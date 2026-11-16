import AppKit
import Foundation

@MainActor
final class CloudController: ObservableObject {
    @Published var phase: WorkstationPhase = .off
    @Published var selectedGPU: String = "G4"
    @Published var activeGPU: String = "G4"
    @Published var eligibleGPUs: [String] = ["G4", "H100", "A100", "L4", "T4"]
    @Published var balance: Double?
    @Published var burnRateHourly: Double = 0
    @Published var accountConnected: Bool = false
    @Published var activeEndpoint: String?
    @Published var activeRegionLabel: String?
    @Published var localTunnelReady: Bool = false
    @Published var lanURLString: String = "http://192.168.1.115:18188"
    @Published var comfyVersionText: String = "ComfyUI 0.45.4，已配置 34 个插件"

    @Published var inventoryState: InventoryLoadState = .loading
    @Published var driveConsentURL: URL?
    @Published var consentOpenedInBrowser: Bool = false
    @Published var currentSubPhase: String = ""
    @Published var logText: String = "尚未读取状态。"
    @Published var errorMessage: String?
    @Published var isBusy: Bool = false
    @Published var isRefreshing: Bool = false
    @Published var showStopConfirm: Bool = false
    @Published var toastMessage: String?

    private let colabctlURL: URL
    private let liveStateURL = URL(fileURLWithPath: "/Volumes/VoltDrive/AI/tmp/comfyui-colab/live-state.json")
    private var livePollTask: Task<Void, Never>?
    private var toastTask: Task<Void, Never>?
    private var activeTask: Task<Void, Never>?
    private var autoOpenedConsentURLs: Set<String> = []

    init() {
        self.colabctlURL = CloudController.resolveColabctl()
        loadManifestSummary()
        Task { await refreshProbe() }
    }

    static func resolveColabctl() -> URL {
        if let env = ProcessInfo.processInfo.environment["COMFY_COLABCTL_PATH"],
           FileManager.default.isExecutableFile(atPath: env) {
            return URL(fileURLWithPath: env)
        }
        // Keep the controller beside the app so the project can be relocated.
        let beside = Bundle.main.bundleURL.deletingLastPathComponent().appendingPathComponent("colabctl")
        return beside
    }

    private func loadManifestSummary() {
        let manifestURL = colabctlURL.deletingLastPathComponent().appendingPathComponent("manifest.json")
        guard let data = try? Data(contentsOf: manifestURL),
              let summary = try? JSONDecoder().decode(ManifestSummary.self, from: data) else { return }
        let ver = summary.comfyui?.version ?? "0.45.4"
        let count = summary.plugins?.count ?? 34
        comfyVersionText = "ComfyUI \(ver)，已配置 \(count) 个插件"
    }

    func showToast(_ message: String) {
        toastMessage = message
        toastTask?.cancel()
        toastTask = Task {
            try? await Task.sleep(nanoseconds: 3_400_000_000)
            if !Task.isCancelled {
                toastMessage = nil
            }
        }
    }

    func gpuSpecs(_ gpu: String) -> (memory: String, title: String, price: String) {
        let dynamicPrice: String? = (phase != .off && burnRateHourly > 0)
            ? String(format: "当前计费 %.2f 点 / 小时", burnRateHourly)
            : nil
        switch gpu.uppercased() {
        case "G4": return ("96 GB 显存", "NVIDIA RTX PRO 6000 Blackwell", dynamicPrice ?? "运行时约 10.05 点 / 小时")
        case "H100": return ("80 GB 显存", "NVIDIA H100", dynamicPrice ?? "费率以实际分配结果为准")
        case "A100": return ("80 GB 显存", "NVIDIA A100-SXM4", dynamicPrice ?? "运行时约 8.88 点 / 小时")
        case "L4": return ("24 GB 显存", "NVIDIA L4", dynamicPrice ?? "费率以实际分配结果为准")
        case "T4": return ("16 GB 显存", "NVIDIA T4", dynamicPrice ?? "费率以实际分配结果为准")
        default: return ("以实例为准", gpu, dynamicPrice ?? "费率以实际分配结果为准")
        }
    }

    var currentInventory: ModelInventory? {
        if case let .loaded(inv) = inventoryState { return inv }
        return nil
    }

    var copyProgress: (copied: Int64, total: Int64, fraction: Double)? {
        guard let inv = currentInventory, !inv.files.isEmpty else { return nil }
        let total = inv.files.reduce(Int64(0)) { $0 + $1.size }
        guard total > 0 else { return nil }
        let copied = inv.files.reduce(Int64(0)) { sum, file in
            let cb = max(Int64(0), min(file.size, file.copiedBytes ?? (file.status == "ready" ? file.size : 0)))
            return sum + cb
        }
        return (copied, total, min(1.0, Double(copied) / Double(total)))
    }

    var activeStepIndex: Int {
        switch phase {
        case .off: return -1
        case .booting:
            switch currentSubPhase {
            case "mounting_drive": return 1
            case "extracting", "syncing_models", "copying_models", "bootstrapping": return 2
            case "starting_comfy": return 3
            default: return 0
            }
        case .auth: return 1
        case .copy:
            return currentSubPhase == "starting_comfy" ? 3 : 2
        case .ready: return 4
        case .stopping: return 4
        case .error: return -1
        }
    }

    func refreshProbe(preserveBootError: String? = nil) async {
        guard !isRefreshing else { return }
        isRefreshing = true
        defer { isRefreshing = false }
        if case .uninitialized = inventoryState {
            inventoryState = .loading
        }
        do {
            let result = try await runColabctl(["probe"])
            let data = extractJSONObject(from: result.stdout) ?? Data(result.stdout.utf8)
            let decoded = try JSONDecoder().decode(ProbeResponse.self, from: data)
            applyProbe(decoded, preserveBootError: preserveBootError)
        } catch {
            if phase != .off {
                phase = .error
            }
            errorMessage = preserveBootError ?? error.localizedDescription
            logText = preserveBootError ?? error.localizedDescription
            if case .loading = inventoryState {
                inventoryState = .failed("无法读取当前状态，请重试。")
            }
        }
    }

    func startWorkstation() {
        guard !isBusy else { return }
        isBusy = true
        errorMessage = nil
        consentOpenedInBrowser = false
        driveConsentURL = nil
        autoOpenedConsentURLs.removeAll()
        phase = .booting
        currentSubPhase = "allocating"
        activeGPU = selectedGPU
        activeRegionLabel = nil
        logText = "正在向 Colab 请求分配 \(selectedGPU) 实例…"
        startLiveStatePolling()

        let targetGPU = selectedGPU
        activeTask = Task {
            defer {
                isBusy = false
                stopLiveStatePolling()
            }
            do {
                let res = try await runColabctl(["up", "--gpu", targetGPU, "--port", "18188"])
                if let data = extractJSONObject(from: res.stdout),
                   let up = try? JSONDecoder().decode(UpResponse.self, from: data) {
                    if up.ready != true && up.drive?.needs_drive_consent == true {
                        if let raw = up.drive?.drive_consent_url, let url = URL(string: raw) {
                            driveConsentURL = url
                            autoOpenConsentIfNeeded(url)
                        }
                        phase = .auth
                        burnRateHourly = burnRateHourly > 0 ? burnRateHourly : (targetGPU == "G4" ? 10.05 : 0)
                        logText = "GPU 已分配，需要你在浏览器中点击允许挂载 Google Drive。"
                        await refreshProbe()
                        return
                    }
                    if up.ready == true {
                        await refreshProbe()
                        showToast("工作站已就绪，可以打开 ComfyUI")
                        return
                    }
                }
                await refreshProbe()
            } catch {
                let errMsg = error.localizedDescription
                phase = .error
                errorMessage = errMsg
                logText = errMsg
                await refreshProbe(preserveBootError: errMsg)
            }
        }
    }

    func openConsentPage() {
        guard let url = driveConsentURL else {
            // Re-run `up` on the same VM to fetch a fresh consent URL if missing.
            startWorkstation()
            return
        }
        NSWorkspace.shared.open(url)
        consentOpenedInBrowser = true
        showToast("请在浏览器中选择 20x 账号并点击「允许」，完成后点「已允许，继续启动」")
    }

    private func autoOpenConsentIfNeeded(_ url: URL) {
        let key = url.absoluteString
        guard !autoOpenedConsentURLs.contains(key) else { return }
        autoOpenedConsentURLs.insert(key)
        NSWorkspace.shared.open(url)
        consentOpenedInBrowser = true
        showToast("已为你打开 Google 授权页，点「允许」后再回来点继续")
    }

    func openComfyUI() {
        guard let targetURL = URL(string: lanURLString) ?? URL(string: "http://127.0.0.1:18188") else { return }
        Task {
            if await checkLocalHTTPReady() {
                NSWorkspace.shared.open(targetURL)
                return
            }
            showToast("检测到本地隧道未连通，正在自动重连…")
            do {
                _ = try await runColabctl(["tunnel", "--port", "18188"])
                localTunnelReady = true
                NSWorkspace.shared.open(targetURL)
            } catch {
                errorMessage = "隧道连接失败：\(error.localizedDescription)"
                phase = .error
            }
        }
    }

    private func checkLocalHTTPReady() async -> Bool {
        guard let u = URL(string: "http://127.0.0.1:18188/system_stats") else { return false }
        var req = URLRequest(url: u, timeoutInterval: 1.8)
        req.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        let cfg = URLSessionConfiguration.ephemeral
        cfg.connectionProxyDictionary = [:]
        let session = URLSession(configuration: cfg)
        do {
            let (_, resp) = try await session.data(for: req)
            return (resp as? HTTPURLResponse)?.statusCode == 200
        } catch {
            return false
        }
    }

    func copyLanURL() {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(lanURLString, forType: .string)
        showToast("已复制 \(lanURLString)")
    }

    func reconnectTunnel() {
        guard !isBusy else { return }
        isBusy = true
        Task {
            defer { isBusy = false }
            do {
                _ = try await runColabctl(["tunnel", "--port", "18188"])
                await refreshProbe()
                showToast("本地与局域网隧道已重连")
            } catch {
                errorMessage = error.localizedDescription
                logText = error.localizedDescription
                phase = .error
            }
        }
    }

    func confirmShutDown() {
        showStopConfirm = false
        activeTask?.cancel()
        stopLiveStatePolling()
        isBusy = true
        phase = .stopping
        logText = "正在将工作流 JSON、设置与 output 图片同步回 Google Drive，随后关闭实例…"
        Task {
            defer { isBusy = false }
            do {
                _ = try await runColabctl(["down"])
                driveConsentURL = nil
                consentOpenedInBrowser = false
                errorMessage = nil
                await refreshProbe()
                showToast("已同步工作流与输出到云盘，实例已关闭停止计费")
            } catch {
                phase = .error
                errorMessage = error.localizedDescription
                logText = error.localizedDescription
            }
        }
    }

    private func startLiveStatePolling() {
        if livePollTask != nil { return }
        livePollTask = Task {
            var tick = 0
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 1_500_000_000)
                if Task.isCancelled { break }
                tick += 1
                var freshLocal = false
                if let data = try? Data(contentsOf: liveStateURL),
                   let live = try? JSONDecoder().decode(LiveStatePayload.self, from: data) {
                    applyLiveState(live)
                    if let updated = live.updatedAt, Int(Date().timeIntervalSince1970) - updated <= 8 {
                        freshLocal = true
                    }
                    if live.phase == "ready" && !isBusy {
                        await refreshProbe()
                        break
                    }
                    if live.phase == "off" {
                        break
                    }
                }
                if !freshLocal && tick % 4 == 0 && !isBusy {
                    await refreshProbe()
                    if phase == .ready || phase == .off { break }
                }
            }
            livePollTask = nil
        }
    }

    private func stopLiveStatePolling() {
        livePollTask?.cancel()
        livePollTask = nil
    }

    private func fallbackPhaseLog() -> String {
        let ep = activeEndpoint.map { " (\($0))" } ?? ""
        switch phase {
        case .off: return "实例已关闭，当前没有正在运行的任务。"
        case .auth: return "等待在浏览器中点击允许挂载 Google Drive\(ep)"
        case .booting:
            return switch currentSubPhase {
            case "mounting_drive": "正在挂载 Google Drive\(ep)…"
            case "extracting": "正在解压恢复 ComfyUI 与 34 个插件环境包\(ep)…"
            case "syncing_models": "正在检查云盘保存的模型目录\(ep)…"
            default: "正在准备云端运行环境\(ep)…"
            }
        case .copy:
            if currentSubPhase == "starting_comfy" {
                return "模型已拷到本地盘，正在启动 ComfyUI 服务\(ep)…"
            }
            if let prog = copyProgress {
                return "正在将模型拷到实例本地盘：\(formatByteCount(prog.copied)) / \(formatByteCount(prog.total))\(ep)"
            }
            return "正在将模型从云盘拷到实例本地盘\(ep)…"
        case .ready:
            return "ComfyUI 已就绪，入口：\(lanURLString)\(ep)"
        case .stopping:
            return "正在同步工作流 JSON 与 output 图片回云盘并关闭实例\(ep)…"
        case .error:
            return errorMessage ?? "连接或启动状态异常\(ep)"
        }
    }

    private func applyLiveState(_ live: LiveStatePayload) {
        if let acc = live.accelerator, !acc.isEmpty { activeGPU = acc }
        if let ep = live.endpoint { activeEndpoint = ep }
        if let reg = live.regionLabel, !reg.isEmpty { activeRegionLabel = reg }
        if let inv = live.inventory { updateInventoryState(inv) }

        switch live.phase {
        case "allocating", "mounting_drive", "bootstrapping", "extracting", "syncing_models":
            phase = .booting
            currentSubPhase = live.phase ?? "bootstrapping"
        case "copying_models", "starting_comfy":
            phase = .copy
            currentSubPhase = live.phase ?? "copying_models"
        case "syncing_shutdown":
            phase = .stopping
            currentSubPhase = "syncing_shutdown"
        case "needs_drive_consent":
            phase = .auth
            if let u = live.driveConsentUrl, let url = URL(string: u) {
                driveConsentURL = url
                autoOpenConsentIfNeeded(url)
            }
        case "ready":
            phase = .ready
            currentSubPhase = "ready"
        case "error":
            phase = .error
            errorMessage = live.error ?? "云端启动异常"
        default:
            break
        }
        if let tail = live.logTail, !tail.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            logText = tail
        } else {
            logText = fallbackPhaseLog()
        }
    }

    private func applyProbe(_ p: ProbeResponse, preserveBootError: String? = nil) {
        balance = p.balance
        burnRateHourly = p.burnRateHourly ?? 0
        if let gpus = p.eligibleGpus, !gpus.isEmpty { eligibleGPUs = gpus }
        if let lan = p.lanUrl, !lan.isEmpty { lanURLString = lan }
        accountConnected = p.accountConnected ?? false
        localTunnelReady = p.localTunnel18188 ?? false

        if let inv = p.inventory {
            updateInventoryState(inv)
        } else {
            inventoryState = .uninitialized
        }

        let instances = p.activeInstances ?? []
        if instances.isEmpty {
            activeEndpoint = nil
            activeRegionLabel = nil
            activeGPU = selectedGPU
            currentSubPhase = ""
            if let bootErr = preserveBootError, !bootErr.isEmpty {
                phase = .error
                errorMessage = bootErr
                logText = bootErr
            } else {
                phase = .off
                errorMessage = nil
                logText = "实例已关闭，当前没有正在运行的任务。"
            }
            return
        }

        let first = instances[0]
        activeEndpoint = first.endpoint
        if let reg = first.regionLabel, !reg.isEmpty {
            activeRegionLabel = reg
        }
        if let acc = first.accelerator, !acc.isEmpty {
            activeGPU = acc
            selectedGPU = acc
        }

        if let remote = p.remote {
            let bsPhase = remote.bootstrap?.phase
            let bsStatus = remote.bootstrap?.status

            if bsStatus == "error" {
                phase = .error
                errorMessage = remote.bootstrap?.error ?? "远端启动失败"
            } else if remote.driveMounted == false {
                if let ls = p.liveState, ls.phase == "needs_drive_consent", let u = ls.driveConsentUrl, let url = URL(string: u) {
                    driveConsentURL = url
                    autoOpenConsentIfNeeded(url)
                }
                phase = .auth
                currentSubPhase = "mounting_drive"
            } else if bsPhase == "copying_models" || bsPhase == "starting_comfy" {
                phase = .copy
                currentSubPhase = bsPhase ?? "copying_models"
            } else if bsPhase == "extracting" || bsPhase == "syncing_models" {
                phase = .booting
                currentSubPhase = bsPhase ?? "extracting"
            } else if bsStatus == "ready" || bsStatus == "already_running" || (remote.modelsLocal == true && localTunnelReady) {
                if localTunnelReady {
                    phase = .ready
                    currentSubPhase = "ready"
                    errorMessage = nil
                } else {
                    phase = .error
                    errorMessage = "云端 ComfyUI 正在运行，但本机 18188 端口未连接，可点「重连隧道」。"
                }
            } else if let ls = p.liveState {
                applyLiveState(ls)
            } else {
                phase = .error
                errorMessage = "实例正在运行，但 ComfyUI 尚未就绪。"
            }

            if let tail = remote.logTail, !tail.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                logText = "\(fallbackPhaseLog())\n\n\(tail)"
            } else if let lsTail = p.liveState?.logTail, !lsTail.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                logText = lsTail
            } else {
                logText = fallbackPhaseLog()
            }
        } else if let ls = p.liveState, ls.phase == "needs_drive_consent" {
            applyLiveState(ls)
        } else {
            phase = .error
            errorMessage = "云端实例正在计费，但 SSH 暂时连不上。"
            logText = fallbackPhaseLog()
        }

        if phase == .booting || phase == .copy {
            startLiveStatePolling()
        }
    }

    private func updateInventoryState(_ inv: ModelInventory) {
        if inv.files.isEmpty {
            inventoryState = .empty(inv.root, cached: inv.cached == true, observedAt: inv.observedAt)
        } else {
            inventoryState = .loaded(inv)
        }
    }

    private func extractJSONObject(from text: String) -> Data? {
        guard let start = text.firstIndex(of: "{"),
              let end = text.lastIndex(of: "}"),
              end >= start else { return nil }
        return Data(text[start...end].utf8)
    }

    private struct CmdResult {
        let code: Int32
        let stdout: String
        let stderr: String
    }

    private func runColabctl(_ args: [String]) async throws -> CmdResult {
        let exe = colabctlURL
        return try await Task.detached(priority: .userInitiated) {
            let proc = Process()
            proc.executableURL = exe
            proc.arguments = args
            proc.currentDirectoryURL = exe.deletingLastPathComponent().deletingLastPathComponent()
            let outPipe = Pipe()
            let errPipe = Pipe()
            proc.standardOutput = outPipe
            proc.standardError = errPipe
            try proc.run()
            proc.waitUntilExit()
            let out = String(data: outPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            let err = String(data: errPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
            if proc.terminationStatus != 0 {
                let msg = err.trimmingCharacters(in: .whitespacesAndNewlines)
                let fallback = out.trimmingCharacters(in: .whitespacesAndNewlines)
                throw NSError(domain: "ComfyCloud", code: Int(proc.terminationStatus), userInfo: [
                    NSLocalizedDescriptionKey: msg.isEmpty ? (fallback.isEmpty ? "命令退出码 \(proc.terminationStatus)" : fallback) : msg
                ])
            }
            return CmdResult(code: proc.terminationStatus, stdout: out, stderr: err)
        }.value
    }
}
