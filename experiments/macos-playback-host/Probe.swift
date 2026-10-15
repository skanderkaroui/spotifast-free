import AppKit
import WebKit

@MainActor
final class PlaybackProbe: NSObject, NSApplicationDelegate, NSWindowDelegate,
    WKNavigationDelegate, WKUIDelegate, WKScriptMessageHandler {
    private let playerURL = URL(string: "https://open.spotify.com")!
    private var window: NSWindow!
    private var webView: WKWebView!
    /// Commands that arrive before Spotify's page has loaded wait here; the
    /// first play after a cold start would otherwise go nowhere.
    private var pending: [String] = []
    private var pageReady = false
    private let status = NSTextField(labelWithString: "Use Spotify's own controls. Diagnostics never read credentials.")

    func applicationDidFinishLaunching(_ notification: Notification) {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
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

        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
                          styleMask: [.titled, .closable, .miniaturizable, .resizable],
                          backing: .buffered, defer: false)
        window.title = "Spotifast Free Playback"
        window.contentView = content
        window.delegate = self
        window.center()
        if CommandLine.arguments.contains("--headless") {
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

    func userContentController(_ userContentController: WKUserContentController,
                               didReceive message: WKScriptMessage) {
        guard let text = message.body as? String else { return }
        FileHandle.standardOutput.write(Data(text.utf8) + Data([10]))
    }

    func windowWillClose(_ notification: Notification) { NSApp.terminate(nil) }
}

MainActor.assumeIsolated {
    let application = NSApplication.shared
    application.setActivationPolicy(.regular)
    let menu = NSMenu()
    let applicationItem = NSMenuItem()
    let applicationMenu = NSMenu()
    applicationMenu.addItem(withTitle: "Quit Playback Probe", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
    applicationItem.submenu = applicationMenu
    menu.addItem(applicationItem)
    application.mainMenu = menu
    let delegate = PlaybackProbe()
    application.delegate = delegate
    application.run()
}
