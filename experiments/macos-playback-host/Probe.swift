import AppKit
import WebKit

@MainActor
final class PlaybackProbe: NSObject, NSApplicationDelegate, NSWindowDelegate,
    WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler, NSToolbarDelegate {
    // Spotify's own settings page: the same web player, with its player bar
    // (ads included) and sign-in, but without the home feed's artwork grids
    // nobody sees in the hidden window. Measured: about 290 MB instead of
    // 370 MB for the page's process while playing.
    private let playerURL = URL(string: "https://open.spotify.com/preferences")!
    private var window: NSWindow!
    private var webView: WKWebView!
    /// Commands that arrive before Spotify's page has loaded wait here; the
    /// first play after a cold start would otherwise go nowhere.
    private var pending: [String] = []
    private var pageReady = false
    private let status = NSTextField(labelWithString: "Use Spotify's own controls. Diagnostics never read credentials.")

    func applicationDidFinishLaunching(_ notification: Notification) {
        let headless = CommandLine.arguments.contains("--headless")
        let configuration = WKWebViewConfiguration()
        // --signed-out: a fresh store that keeps nothing, to see the sign-in
        // window without signing the real one out.
        configuration.websiteDataStore = CommandLine.arguments.contains("--signed-out")
            ? .nonPersistent() : .default()
        // A music player has no reason to block audio. With the default
        // gesture rule, Spotify's play() runs after async work, loses the
        // click, and WebKit refuses it ("PageConsentRequired").
        configuration.mediaTypesRequiringUserActionForPlayback = []
        if let url = Bundle.main.url(forResource: "Diagnostics", withExtension: "js"),
           let source = try? String(contentsOf: url, encoding: .utf8) {
            configuration.userContentController.addUserScript(WKUserScript(
                source: source, injectionTime: .atDocumentStart, forMainFrameOnly: false))
        }
        configuration.userContentController.add(self, name: "probe")
        // WKWebView's default user agent omits Safari's "Version/x Safari/y"
        // suffix, so sites treat it as an unknown browser. Append the suffix
        // of the Safari installed on this Mac; the engine is the same WebKit.
        if let safari = Bundle(path: "/Applications/Safari.app"),
           let version = safari.infoDictionary?["CFBundleShortVersionString"] as? String {
            configuration.applicationNameForUserAgent = "Version/\(version) Safari/605.1.15"
        }
        webView = WKWebView(frame: .zero, configuration: configuration)
        webView.isInspectable = true
        webView.navigationDelegate = self
        webView.uiDelegate = self

        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
        if headless {
            // The app's helper only ever shows this window for Spotify's
            // sign-in: Spotify's page alone, without the probe's tools.
            window.title = "Sign in to Spotify"
            window.contentView = webView
            browserChrome()
        } else {
            window.title = "Spotifast Free Playback"
            window.contentView = probeTools()
        }
        window.delegate = self
        window.center()
        if headless {
            // Background helper: no Dock icon, window never shown. Spotifast
            // drives it through stdin and reads events from stdout.
            NSApp.setActivationPolicy(.accessory)
            startCommandReader()
            if CommandLine.arguments.contains("--web-player") { openPlayer() }
            return
        }
        window.makeKeyAndOrderFront(nil)
        NSApp.activate()
        openPlayer()
    }

    // The sign-in window's browser bar, laid out as Safari's compact one:
    // back and forward by the window buttons, the address in the middle
    // with its lock, reload at its end and a loading line under it.
    private var backItem: NSToolbarItem?
    private var forwardItem: NSToolbarItem?
    private let address = AddressField()
    private var watching: [NSKeyValueObservation] = []

    private func browserChrome() {
        address.reload.target = self
        address.reload.action = #selector(reloadOrStop)
        let toolbar = NSToolbar(identifier: "SignIn")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        toolbar.centeredItemIdentifiers = [.address]
        window.toolbar = toolbar
        window.toolbarStyle = .unifiedCompact
        window.titleVisibility = .hidden
        let refresh: (WKWebView) -> Void = { [weak self] _ in
            Task { @MainActor in self?.refreshBar() }
        }
        watching = [
            webView.observe(\.url) { view, _ in refresh(view) },
            webView.observe(\.title) { view, _ in refresh(view) },
            webView.observe(\.canGoBack) { view, _ in refresh(view) },
            webView.observe(\.canGoForward) { view, _ in refresh(view) },
            webView.observe(\.isLoading) { view, _ in refresh(view) },
            webView.observe(\.estimatedProgress) { view, _ in refresh(view) },
        ]
        refreshBar()
    }

    private func refreshBar() {
        backItem?.isEnabled = webView.canGoBack
        forwardItem?.isEnabled = webView.canGoForward
        address.show(url: webView.url, loading: webView.isLoading, progress: webView.estimatedProgress)
        if let title = webView.title, !title.isEmpty { window.title = title }
    }

    @objc private func reloadOrStop() {
        if webView.isLoading { webView.stopLoading() } else { webView.reload() }
    }

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.back, .forward, .flexibleSpace, .address, .flexibleSpace]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(_ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
                 willBeInsertedIntoToolbar flag: Bool) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        switch identifier {
        case .back, .forward:
            let back = identifier == .back
            item.label = back ? "Back" : "Forward"
            item.toolTip = back ? "Show the previous page" : "Show the next page"
            item.image = NSImage(systemSymbolName: back ? "chevron.left" : "chevron.right",
                                 accessibilityDescription: item.label)
            item.isNavigational = true
            item.autovalidates = false
            item.target = webView
            item.action = back ? #selector(WKWebView.goBack(_:)) : #selector(WKWebView.goForward(_:))
            if back { backItem = item } else { forwardItem = item }
        case .address:
            item.label = "Address"
            item.view = address
        default:
            return nil
        }
        return item
    }

    /// The probe window's own tools around Spotify's page, for running it
    /// by hand: open the player, check media support, and a status line.
    private func probeTools() -> NSView {
        let buttons = NSStackView(views: [
            NSButton(title: "Open Web Player", target: self, action: #selector(openPlayer)),
            NSButton(title: "Check Media Support", target: self, action: #selector(checkMedia)),
        ])
        buttons.orientation = .horizontal
        status.lineBreakMode = .byTruncatingTail
        let content = NSStackView(views: [buttons, webView, status])
        content.orientation = .vertical
        content.alignment = .leading
        content.spacing = 12
        content.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
        webView.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            webView.widthAnchor.constraint(equalTo: content.widthAnchor, constant: -24),
            webView.heightAnchor.constraint(greaterThanOrEqualToConstant: 350),
            status.widthAnchor.constraint(equalTo: webView.widthAnchor),
        ])
        return content
    }

    @objc private func openPlayer() { webView.load(URLRequest(url: playerURL)) }

    /// One JSON object per line: {"cmd":"connect","action":...,"args":{...}},
    /// {"cmd":"media","action":...} or {"cmd":"quit"}.
    private func startCommandReader() {
        Thread.detachNewThread { [weak self] in
            while let line = readLine() {
                Task { @MainActor in self?.handle(line) }
            }
            Task { @MainActor in NSApp.terminate(nil) }
        }
    }

    private func handle(_ line: String) {
        // Quit never waits for the page: a page that never loads (offline)
        // would otherwise hold the helper open forever.
        if line.contains("\"quit\""),
           let data = line.data(using: .utf8),
           let command = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           command["cmd"] as? String == "quit" {
            NSApp.terminate(nil)
            return
        }
        guard pageReady else {
            pending.append(line)
            return
        }
        guard let data = line.data(using: .utf8),
              let command = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let name = command["cmd"] as? String else { return }
        switch name {
        case "connect":
            // {"cmd":"connect","action":"play"|"pause"|...,"args":{...}}:
            // one Spotify Connect request, sent by the page itself with its
            // own sign-in (see Diagnostics.js). Arguments go in as data,
            // never spliced into script text.
            guard let action = command["action"] as? String else { return }
            let args = command["args"] as? [String: Any] ?? [:]
            webView.callAsyncJavaScript(
                "await window.__spotifastConnect?.(action, args)",
                arguments: ["action": action, "args": args], in: nil, in: .page) { _ in }
        case "query":
            // {"cmd":"query","id":N,"query":{...}}: one read of Spotify's web
            // player API, sent by the page with its own sign-in; the answer
            // comes back as {"event":"query","id":N,...}.
            guard let id = command["id"], let query = command["query"] as? [String: Any] else { return }
            webView.callAsyncJavaScript(
                "await window.__spotifastQuery?.(id, query)",
                arguments: ["id": id, "query": query], in: nil, in: .page) { _ in }
        case "media":
            // Media Session actions Spotify registered: play, pause,
            // nexttrack, previoustrack. Used by checks; Spotifast sends
            // "connect".
            let actions: Set<String> = ["play", "pause", "nexttrack", "previoustrack"]
            guard let action = command["action"] as? String, actions.contains(action) else { return }
            webView.callAsyncJavaScript(
                "window.__spotifastMediaSession?.(action, {})",
                arguments: ["action": action], in: nil, in: .page) { _ in }
        case "quit":
            NSApp.terminate(nil)
        case "snapshot":
            // {"cmd":"snapshot","path":"/x.png"}: the window as it shows,
            // bar and page, for checking the sign-in window without a screen
            // capture. The page draws out of process, so it is snapshotted
            // on its own and laid over the window's frame.
            guard let path = command["path"] as? String,
                  let frame = window.contentView?.superview else { return }
            frame.layoutSubtreeIfNeeded()
            webView.takeSnapshot(with: nil) { [weak self] page, _ in
                guard let self, let page,
                      let bitmap = frame.bitmapImageRepForCachingDisplay(in: frame.bounds) else { return }
                frame.cacheDisplay(in: frame.bounds, to: bitmap)
                let image = NSImage(size: frame.bounds.size)
                image.addRepresentation(bitmap)
                image.lockFocus()
                page.draw(in: self.webView.convert(self.webView.bounds, to: frame))
                image.unlockFocus()
                guard let tiff = image.tiffRepresentation,
                      let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:])
                else { return }
                try? png.write(to: URL(fileURLWithPath: path))
                if let note = try? JSONSerialization.data(withJSONObject: ["event": "snapshot", "path": path]) {
                    FileHandle.standardOutput.write(note + Data([10]))
                }
            }
        default:
            break
        }
    }

    @objc private func checkMedia() {
        guard let scriptURL = Bundle.main.url(forResource: "MediaCapabilities", withExtension: "js"),
              let script = try? String(contentsOf: scriptURL, encoding: .utf8) else {
            finish(["event": "probe-error", "reason": "missing-capability-script"])
            return
        }
        webView.callAsyncJavaScript(script, arguments: [:], in: nil, in: .page) { [weak self] result in
            guard let self else { return }
            switch result {
            case .success(let value):
                guard let text = value as? String,
                      let data = text.data(using: .utf8),
                      let report = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                    self.finish(["event": "probe-error", "reason": "invalid-capability-report"])
                    return
                }
                self.status.stringValue = "EME: \(report["eme"] ?? false). MediaSource: \(report["mediaSource"] ?? false). See the diagnostic JSON."
                self.finish(["event": "media-capabilities", "report": report])
            case .failure(let error):
                let error = error as NSError
                self.finish(["event": "probe-error", "domain": error.domain, "code": error.code])
            }
        }
    }

    private func finish(_ report: [String: Any]) {
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.sortedKeys]) {
            FileHandle.standardOutput.write(data + Data([10]))
        }
        if CommandLine.arguments.contains("--quit-after-probe") { NSApp.terminate(nil) }
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        checkMedia()
        // Spotify's player registers its controls a moment after the page
        // loads; give it that before replaying what waited.
        if !pageReady {
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                guard let self else { return }
                self.pageReady = true
                let waiting = self.pending
                self.pending.removeAll()
                waiting.forEach(self.handle)
            }
        }
    }

    func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!,
                 withError error: Error) {
        let error = error as NSError
        status.stringValue = "Navigation failed (\(error.domain), \(error.code))."
        finish(["event": "navigation-error", "domain": error.domain, "code": error.code])
    }

    // Spotify's normal sign-in links may target a new window. Keep that flow in this store.
    func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                 for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
        if navigationAction.targetFrame == nil { webView.load(navigationAction.request) }
        return nil
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        // Spotifast's hidden helper: closing its sign-in window hides it
        // rather than quitting the player Spotifast is talking to.
        guard CommandLine.arguments.contains("--headless") else { return true }
        sender.orderOut(nil)
        NSApp.setActivationPolicy(.accessory)
        return false
    }

    func userContentController(_ userContentController: WKUserContentController,
                               didReceive message: WKScriptMessage) {
        guard let text = message.body as? String else { return }
        // Spotify dropped this page's device (its live connection was lost)
        // and the page could not get it back. Reload Spotify's page, which
        // registers it again, and replay the press that found it gone, once.
        // The refusal itself is not passed on: the replay answers for it.
        if text.contains("\"how\":\"device-lost\""), recoverLostDevice(from: text) { return }
        FileHandle.standardOutput.write(Data(text.utf8) + Data([10]))
        if text.contains("\"event\":\"account\""),
           let data = text.data(using: .utf8),
           let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
           let state = event["state"] as? String {
            account(state)
        }
    }

    /// When the page was last reloaded to get its device back. One reload a
    /// minute at most: if Spotify still has no device after that (the
    /// network is down), the refusal goes through to Spotifast.
    private var lastRecovery: Date?

    /// Reloads Spotify's page and queues the refused command (its own action
    /// and arguments, as the page reported them) to run once it is ready.
    private func recoverLostDevice(from text: String) -> Bool {
        if let last = lastRecovery, Date().timeIntervalSince(last) < 60 { return false }
        guard let data = text.data(using: .utf8),
              let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let action = event["action"] as? String else { return false }
        let replay: [String: Any] = ["cmd": "connect", "action": action, "args": event["args"] ?? [:]]
        guard let line = try? JSONSerialization.data(withJSONObject: replay),
              let command = String(data: line, encoding: .utf8) else { return false }
        lastRecovery = Date()
        if let note = try? JSONSerialization.data(withJSONObject: ["event": "device-recovery", "action": action]) {
            FileHandle.standardOutput.write(note + Data([10]))
        }
        pageReady = false
        pending.insert(command, at: 0)
        webView.reload()
        return true
    }

    /// Spotify's sign-in, as the page reports it. Signed out: show the
    /// window so the user can sign in on Spotify's own page. Signed in after
    /// that: hide it again (Spotifast's hidden helper) or close it (the
    /// sign-in window opened by hand), with nothing for the user to confirm.
    private var askedToSignIn = false
    private func account(_ state: String) {
        let headless = CommandLine.arguments.contains("--headless")
        switch state {
        case "signed-out":
            askedToSignIn = true
            status.stringValue = "Sign in to Spotify here. This window closes by itself once you are in."
            if headless {
                // Spotify's own login page alone, rather than the web player
                // behind a Log in button. It returns to the player page once
                // signed in (the web player's own "continue" link).
                var login = URLComponents(string: "https://accounts.spotify.com/login")!
                login.queryItems = [URLQueryItem(name: "continue", value: playerURL.absoluteString)]
                if let url = login.url { webView.load(URLRequest(url: url)) }
                NSApp.setActivationPolicy(.regular)
                window.makeKeyAndOrderFront(nil)
                NSApp.activate()
            }
        case "signed-in" where askedToSignIn:
            askedToSignIn = false
            if headless {
                window.orderOut(nil)
                NSApp.setActivationPolicy(.accessory)
                // Spotify's sign-in returns to its home feed. Go back to the
                // lighter page; it registers this account's player as it
                // loads, and commands wait for it.
                if webView.url?.path != playerURL.path {
                    pageReady = false
                    webView.load(URLRequest(url: playerURL))
                }
            } else {
                NSApp.terminate(nil)
            }
        default:
            break
        }
    }

    func windowWillClose(_ notification: Notification) { NSApp.terminate(nil) }
}

extension NSToolbarItem.Identifier {
    static let back = Self("back")
    static let forward = Self("forward")
    static let address = Self("address")
}

/// Safari's address field, read-only: the lock and the site, centred, with
/// reload at its end and the page's loading progress along its bottom.
final class AddressField: NSView {
    let reload = NSButton()
    private let lock = NSImageView()
    private let host = NSTextField(labelWithString: "")
    private let progress = NSView()
    private var progressWidth: NSLayoutConstraint!

    override var wantsUpdateLayer: Bool { true }
    override func updateLayer() {
        layer?.backgroundColor = NSColor.quaternaryLabelColor.cgColor
        progress.layer?.backgroundColor = NSColor.controlAccentColor.cgColor
    }

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.cornerRadius = 7
        layer?.masksToBounds = true
        setAccessibilityRole(.group)
        setAccessibilityLabel("Address")
        lock.image = NSImage(systemSymbolName: "lock.fill", accessibilityDescription: "Secure")
        lock.symbolConfiguration = .init(pointSize: 10, weight: .medium)
        lock.contentTintColor = .secondaryLabelColor
        host.font = .systemFont(ofSize: 13)
        host.lineBreakMode = .byTruncatingMiddle
        reload.isBordered = false
        reload.imagePosition = .imageOnly
        reload.symbolConfiguration = .init(pointSize: 11, weight: .medium)
        reload.contentTintColor = .secondaryLabelColor
        progress.wantsLayer = true
        let site = NSStackView(views: [lock, host])
        site.spacing = 5
        for view in [site, reload, progress] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        progressWidth = progress.widthAnchor.constraint(equalToConstant: 0)
        NSLayoutConstraint.activate([
            widthAnchor.constraint(equalToConstant: 420),
            heightAnchor.constraint(equalToConstant: 28),
            site.centerXAnchor.constraint(equalTo: centerXAnchor),
            site.centerYAnchor.constraint(equalTo: centerYAnchor),
            site.trailingAnchor.constraint(lessThanOrEqualTo: reload.leadingAnchor, constant: -6),
            reload.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            reload.centerYAnchor.constraint(equalTo: centerYAnchor),
            progress.leadingAnchor.constraint(equalTo: leadingAnchor),
            progress.bottomAnchor.constraint(equalTo: bottomAnchor),
            progress.heightAnchor.constraint(equalToConstant: 2),
            progressWidth,
        ])
        show(url: nil, loading: false, progress: 0)
    }

    required init?(coder: NSCoder) { nil }

    func show(url: URL?, loading: Bool, progress value: Double) {
        host.stringValue = url?.host() ?? ""
        lock.isHidden = url?.scheme != "https"
        toolTip = url?.absoluteString
        reload.image = NSImage(systemSymbolName: loading ? "xmark" : "arrow.clockwise",
                               accessibilityDescription: loading ? "Stop" : "Reload")
        reload.toolTip = loading ? "Stop loading this page" : "Reload this page"
        progress.isHidden = !loading
        progressWidth.constant = 420 * min(max(value, 0), 1)
    }
}

MainActor.assumeIsolated {
    let application = NSApplication.shared
    application.setActivationPolicy(.regular)
    let menu = NSMenu()
    let applicationItem = NSMenuItem()
    let applicationMenu = NSMenu()
    applicationMenu.addItem(withTitle: "Quit Spotifast Free Helper", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
    applicationItem.submenu = applicationMenu
    menu.addItem(applicationItem)
    // A browser's own shortcuts: paste an email or password, close the
    // window, reload, back and forward.
    func submenu(_ title: String, _ items: [(String, Selector, String)]) {
        let item = NSMenuItem()
        let submenu = NSMenu(title: title)
        for (name, action, key) in items {
            submenu.addItem(withTitle: name, action: action, keyEquivalent: key)
        }
        item.submenu = submenu
        menu.addItem(item)
    }
    submenu("File", [("Close Window", #selector(NSWindow.performClose(_:)), "w")])
    submenu("Edit", [
        ("Undo", Selector(("undo:")), "z"),
        ("Redo", Selector(("redo:")), "Z"),
        ("Cut", #selector(NSText.cut(_:)), "x"),
        ("Copy", #selector(NSText.copy(_:)), "c"),
        ("Paste", #selector(NSText.paste(_:)), "v"),
        ("Select All", #selector(NSText.selectAll(_:)), "a"),
    ])
    submenu("View", [("Reload Page", #selector(WKWebView.reload(_:)), "r")])
    submenu("History", [
        ("Back", #selector(WKWebView.goBack(_:)), "["),
        ("Forward", #selector(WKWebView.goForward(_:)), "]"),
    ])
    application.mainMenu = menu
    let delegate = PlaybackProbe()
    application.delegate = delegate
    application.run()
}
