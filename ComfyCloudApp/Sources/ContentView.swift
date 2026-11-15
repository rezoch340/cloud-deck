import SwiftUI

private enum Palette {
    static let sidebar = Color(red: 0.941, green: 0.945, blue: 0.965) // #F0F1F6
    static let paper = Color(red: 0.988, green: 0.988, blue: 0.996)   // #FCFCFE
    static let cardBottom = Color(red: 0.973, green: 0.976, blue: 0.988) // #F8F9FC
    static let ink = Color(red: 0.145, green: 0.161, blue: 0.212)     // #252936
    static let muted = Color(red: 0.384, green: 0.412, blue: 0.471)   // #626978
    static let line = Color(red: 0.882, green: 0.894, blue: 0.929)    // #E1E4ED
    static let accent = Color(red: 0.325, green: 0.357, blue: 0.835)  // #535BD5
    static let navActive = Color(red: 0.875, green: 0.886, blue: 0.953) // #DFE2F3
    static let navActiveInk = Color(red: 0.224, green: 0.267, blue: 0.643)
    static let green = Color(red: 0.137, green: 0.443, blue: 0.345)   // #237158
    static let greenBg = Color(red: 0.918, green: 0.961, blue: 0.937) // #EAF5EF
    static let amber = Color(red: 0.545, green: 0.369, blue: 0.086)   // #8B5E16
    static let amberBg = Color(red: 0.988, green: 0.957, blue: 0.902) // #FCF4E6
    static let red = Color(red: 0.690, green: 0.267, blue: 0.267)     // #B04444
    static let redBg = Color(red: 0.988, green: 0.941, blue: 0.941)   // #FCF0F0
    static let neutralBadge = Color(red: 0.953, green: 0.957, blue: 0.969)
}

struct ContentView: View {
    @ObservedObject var ctrl: CloudController
    @State private var nav: NavSection = .workstation

    var body: some View {
        HStack(spacing: 0) {
            sidebar
            Divider().overlay(Palette.line)
            mainWorkspace
        }
        .background(Palette.paper)
        .preferredColorScheme(.light)
        .frame(minWidth: 1020, idealWidth: 1060, minHeight: 710, idealHeight: 740)
        .overlay(alignment: .bottom) {
            if let msg = ctrl.toastMessage {
                Text(msg)
                    .font(.system(size: 12.5, weight: .medium))
                    .foregroundColor(.white)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 10)
                    .background(Color(red: 0.188, green: 0.216, blue: 0.298), in: RoundedRectangle(cornerRadius: 10))
                    .shadow(color: .black.opacity(0.18), radius: 12, y: 4)
                    .padding(.bottom, 22)
                    .transition(.opacity)
            }
        }
        .alert("关机并停止计费？", isPresented: $ctrl.showStopConfirm) {
            Button("继续运行", role: .cancel) {}
            Button("同步并关机", role: .destructive) { ctrl.confirmShutDown() }
        } message: {
            Text("关机前会自动将本次修改的工作流 JSON、设置以及新生成的 output 图片同步回 Google Drive，等待刷盘完成后删除实例并停止计费。")
        }
    }

    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            Spacer().frame(height: 36)

            HStack(spacing: 10) {
                RoundedRectangle(cornerRadius: 10, style: .continuous)
                    .fill(Palette.accent)
                    .frame(width: 35, height: 35)
                    .overlay {
                        Image(systemName: "icloud.fill")
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundColor(.white)
                    }
                    .shadow(color: Palette.accent.opacity(0.22), radius: 6, y: 3)

                VStack(alignment: .leading, spacing: 2) {
                    Text("Comfy Cloud")
                        .font(.system(size: 15.5, weight: .semibold))
                        .foregroundColor(Palette.ink)
                    Text("你的云端创作工作站")
                        .font(.system(size: 11))
                        .foregroundColor(Palette.muted)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 26)

            Text("工作空间")
                .font(.system(size: 11))
                .foregroundColor(Palette.muted)
                .padding(.horizontal, 20)
                .padding(.bottom, 8)

            VStack(spacing: 4) {
                navButton(.workstation, icon: "square.grid.2x2")
                navButton(.models, icon: "shippingbox", count: modelCountBadge)
                navButton(.logs, icon: "doc.text")
            }
            .padding(.horizontal, 12)

            Spacer()

            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    Circle()
                        .fill(Color.white)
                        .frame(width: 30, height: 30)
                        .overlay(Circle().stroke(Palette.line, lineWidth: 1))
                        .overlay {
                            Image(systemName: "person.crop.circle.badge.checkmark")
                                .font(.system(size: 13))
                                .foregroundColor(Palette.accent)
                        }
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Google AI Ultra")
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundColor(Palette.ink)
                        Text(ctrl.accountConnected ? "已连接 Google 账号" : "未找到本地登录凭证")
                            .font(.system(size: 10.5))
                            .foregroundColor(Palette.muted)
                    }
                }

                Divider().overlay(Palette.line)

                HStack {
                    Text("可用计算点数")
                        .font(.system(size: 11))
                        .foregroundColor(Palette.muted)
                    Spacer()
                    Text(ctrl.balance.map { String(format: "%.2f", $0) } ?? "—")
                        .font(.system(size: 15, weight: .semibold).monospacedDigit())
                        .foregroundColor(Palette.ink)
                }

                Text(ctrl.burnRateHourly > 0
                     ? String(format: "当前消耗约 %.2f 点 / 小时", ctrl.burnRateHourly)
                     : "当前未分配机器 · 0 点/小时")
                    .font(.system(size: 10))
                    .foregroundColor(ctrl.burnRateHourly > 0 ? Palette.amber : Palette.muted)
            }
            .padding(18)
        }
        .frame(width: 210)
        .background(Palette.sidebar)
    }

    private func navButton(_ section: NavSection, icon: String, count: String? = nil) -> some View {
        let selected = nav == section
        return Button {
            nav = section
        } label: {
            HStack(spacing: 9) {
                Image(systemName: icon)
                    .font(.system(size: 13.5))
                Text(section.rawValue)
                    .font(.system(size: 13, weight: selected ? .semibold : .regular))
                Spacer()
                if let count {
                    Text(count)
                        .font(.system(size: 10.5, weight: .medium).monospacedDigit())
                        .foregroundColor(Palette.muted)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Palette.line.opacity(0.8), in: RoundedRectangle(cornerRadius: 5))
                }
            }
            .foregroundColor(selected ? Palette.navActiveInk : Palette.muted)
            .padding(.horizontal, 12)
            .frame(height: 38)
            .background(selected ? Palette.navActive : Color.clear, in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
    }

    private var mainWorkspace: some View {
        VStack(spacing: 0) {
            HStack {
                Text(nav.rawValue)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(Palette.ink)
                Spacer()
                Button {
                    Task { await ctrl.refreshProbe() }
                } label: {
                    Label(ctrl.isRefreshing ? "刷新中…" : "刷新状态", systemImage: "arrow.clockwise")
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                }
                .buttonStyle(.plain)
                .disabled(ctrl.isRefreshing)
                .keyboardShortcut("r", modifiers: .command)
            }
            .padding(.horizontal, 28)
            .padding(.top, 6)
            .frame(height: 54)
            .background(Palette.paper)
            .overlay(alignment: .bottom) { Divider().overlay(Palette.line.opacity(0.7)) }

            Group {
                switch nav {
                case .workstation:
                    workstationPage
                case .models:
                    modelsOnlyPage
                case .logs:
                    logsOnlyPage
                }
            }
            .padding(.horizontal, 30)
            .padding(.vertical, 22)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            Divider().overlay(Palette.line.opacity(0.8))
            HStack {
                Button {
                    if ctrl.localTunnelReady { ctrl.openComfyUI() } else { ctrl.copyLanURL() }
                } label: {
                    Label(
                        "局域网入口 \(ctrl.lanURLString.replacingOccurrences(of: "http://", with: ""))",
                        systemImage: "link"
                    )
                    .font(.system(size: 11))
                    .foregroundColor(Palette.muted)
                }
                .buttonStyle(.plain)

                Spacer()
                Text(footerStatusText)
                    .font(.system(size: 11))
                    .foregroundColor(ctrl.localTunnelReady ? Palette.green : Palette.muted)
            }
            .padding(.horizontal, 28)
            .frame(height: 36)
            .background(Palette.paper)
        }
    }

    private var workstationPage: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("云端工作站")
                        .font(.system(size: 23, weight: .semibold))
                        .foregroundColor(Palette.ink)
                    Text(ctrl.comfyVersionText)
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                }
                Spacer()
                statusBadge
            }

            machineCard

            HStack(spacing: 8) {
                Image(systemName: "checkmark.shield")
                    .font(.system(size: 12))
                    .foregroundColor(Palette.muted)
                Text(inlineNoteText)
                    .font(.system(size: 11.5))
                    .foregroundColor(Palette.muted)
            }

            HStack(alignment: .top, spacing: 24) {
                inventorySection
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                Rectangle()
                    .fill(Palette.line)
                    .frame(width: 1)
                    .padding(.vertical, 2)
                stepsSection
                    .frame(width: 196, alignment: .topLeading)
            }
            .frame(maxHeight: .infinity, alignment: .top)

            Divider().overlay(Palette.line)

            Button {
                nav = .logs
            } label: {
                HStack(spacing: 10) {
                    Label("运行日志", systemImage: "doc.text")
                        .font(.system(size: 11.5, weight: .medium))
                        .foregroundColor(Palette.muted)
                    Spacer()
                    Text(logSummaryText)
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundColor(Palette.muted)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundColor(Palette.muted.opacity(0.7))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, 2)
        }
    }

    private var modelsOnlyPage: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("模型与工作流")
                        .font(.system(size: 22, weight: .semibold))
                        .foregroundColor(Palette.ink)
                    Text("实时读取 Google Drive 与当前实例本地盘模型状态。")
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                }
                Spacer()
                Button {
                    Task { await ctrl.refreshProbe() }
                } label: {
                    Label("重新核对", systemImage: "arrow.clockwise")
                        .font(.system(size: 12))
                }
            }
            inventorySection
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private var logsOnlyPage: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("运行记录")
                        .font(.system(size: 22, weight: .semibold))
                        .foregroundColor(Palette.ink)
                    Text(logPageSubtitle)
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                }
                Spacer()
                Button("复制日志") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(ctrl.logText, forType: .string)
                    ctrl.showToast("已复制日志")
                }
            }
            ScrollViewReader { proxy in
                ScrollView {
                    Text(ctrl.logText)
                        .font(.system(size: 11.5, design: .monospaced))
                        .foregroundColor(Palette.ink)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(14)
                        .id("LOG_BOTTOM")
                }
                .scrollIndicators(.never)
                .onChange(of: ctrl.logText) { _, _ in
                    proxy.scrollTo("LOG_BOTTOM", anchor: .bottom)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Palette.sidebar, in: RoundedRectangle(cornerRadius: 10))
        }
    }

    private var statusBadge: some View {
        let (label, fg, bg, border) = badgeStyle
        return HStack(spacing: 6) {
            Circle().fill(fg).frame(width: 6, height: 6)
            Text(label)
                .font(.system(size: 11, weight: .medium))
                .foregroundColor(fg)
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 6)
        .background(bg, in: Capsule())
        .overlay(Capsule().stroke(border, lineWidth: 1))
    }

    private var machineCard: some View {
        let gpuName = ctrl.phase == .off ? ctrl.selectedGPU : ctrl.activeGPU
        let specs = ctrl.gpuSpecs(gpuName)
        return VStack(spacing: 0) {
            HStack(spacing: 18) {
                RoundedRectangle(cornerRadius: 16, style: .continuous)
                    .fill(Color(red: 0.945, green: 0.949, blue: 0.976))
                    .frame(width: 68, height: 68)
                    .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(Palette.line, lineWidth: 1))
                    .overlay {
                        VStack(spacing: 1) {
                            Image(systemName: "cpu")
                                .font(.system(size: 20, weight: .light))
                                .foregroundColor(Palette.accent.opacity(0.75))
                            Text(gpuName)
                                .font(.system(size: 11.5, weight: .bold))
                                .foregroundColor(Palette.navActiveInk)
                        }
                    }

                VStack(alignment: .leading, spacing: 5) {
                    HStack(spacing: 8) {
                        Text(gpuName)
                            .font(.system(size: 19, weight: .semibold))
                            .foregroundColor(Palette.ink)
                        Text(specs.memory)
                            .font(.system(size: 11))
                            .foregroundColor(Palette.muted)
                            .padding(.horizontal, 7)
                            .padding(.vertical, 2)
                            .background(Palette.sidebar, in: RoundedRectangle(cornerRadius: 5))

                        if ctrl.phase != .off, let region = ctrl.activeRegionLabel, !region.isEmpty {
                            HStack(spacing: 4) {
                                Image(systemName: "location.fill")
                                    .font(.system(size: 9.5))
                                Text(region)
                                    .font(.system(size: 11, weight: .medium))
                            }
                            .foregroundColor(Palette.navActiveInk)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 2.5)
                            .background(Palette.navActive.opacity(0.65), in: RoundedRectangle(cornerRadius: 5))
                        }
                    }
                    Text(specs.title)
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                    Text(specs.price)
                        .font(.system(size: 12, weight: .medium))
                        .foregroundColor(Palette.ink)
                }

                Spacer()

                VStack(alignment: .leading, spacing: 6) {
                    Text("下次启动使用")
                        .font(.system(size: 10.5))
                        .foregroundColor(Palette.muted)
                    Picker("", selection: $ctrl.selectedGPU) {
                        ForEach(ctrl.eligibleGPUs, id: \.self) { gpu in
                            Text(gpu).tag(gpu)
                        }
                    }
                    .labelsHidden()
                    .frame(width: 108)
                    .disabled((ctrl.phase != .off && !(ctrl.phase == .error && ctrl.activeEndpoint == nil)) || ctrl.isBusy)
                }
            }
            .padding(.horizontal, 22)
            .padding(.vertical, 18)
            .background(Color.white)

            Divider().overlay(Palette.line)

            HStack(spacing: 16) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(stateTitle)
                        .font(.system(size: 15.5, weight: .semibold))
                        .foregroundColor(Palette.ink)
                    Text(stateDescription)
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                        .fixedSize(horizontal: false, vertical: true)

                    if ctrl.phase == .copy || (ctrl.phase == .booting && ctrl.currentSubPhase == "copying_models"),
                       let prog = ctrl.copyProgress {
                        ProgressView(value: prog.fraction)
                            .tint(Palette.accent)
                            .frame(maxWidth: 360)
                            .padding(.top, 4)
                    }
                }

                Spacer()

                HStack(spacing: 10) {
                    if ctrl.phase == .auth && ctrl.driveConsentURL != nil {
                        Button {
                            ctrl.openConsentPage()
                        } label: {
                            Text("重开授权页")
                                .font(.system(size: 12))
                                .foregroundColor(Palette.muted)
                                .padding(.horizontal, 12)
                                .frame(height: 36)
                                .background(Color.white, in: RoundedRectangle(cornerRadius: 7))
                                .overlay(RoundedRectangle(cornerRadius: 7).stroke(Palette.line, lineWidth: 1))
                        }
                        .buttonStyle(.plain)
                    }

                    if ctrl.phase != .off && ctrl.phase != .stopping && !(ctrl.phase == .error && ctrl.activeEndpoint == nil) {
                        Button {
                            ctrl.showStopConfirm = true
                        } label: {
                            Text("关机")
                                .font(.system(size: 12.5, weight: .medium))
                                .foregroundColor(Palette.red)
                                .padding(.horizontal, 14)
                                .frame(height: 36)
                                .background(Color.white, in: RoundedRectangle(cornerRadius: 7))
                                .overlay(RoundedRectangle(cornerRadius: 7).stroke(Palette.line, lineWidth: 1))
                        }
                        .buttonStyle(.plain)
                        .disabled(ctrl.isBusy && ctrl.phase != .booting && ctrl.phase != .copy && ctrl.phase != .auth)
                    }

                    primaryActionButton
                }
            }
            .padding(.horizontal, 22)
            .padding(.vertical, 18)
            .background(Palette.cardBottom)
        }
        .clipShape(RoundedRectangle(cornerRadius: 13, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 13, style: .continuous).stroke(Palette.line, lineWidth: 1))
    }

    @ViewBuilder
    private var primaryActionButton: some View {
        let (title, icon, disabled): (String, String, Bool) = switch ctrl.phase {
        case .off: ("启动工作站", "power", ctrl.isBusy)
        case .booting: ("正在启动…", "hourglass", true)
        case .auth: (ctrl.consentOpenedInBrowser ? "已允许，继续启动" : "打开授权页面", "arrow.up.right", ctrl.isBusy)
        case .copy: ("正在拉取…", "arrow.down.circle", true)
        case .ready: ("打开 ComfyUI", "arrow.up.right", false)
        case .stopping: ("同步关机中…", "icloud.and.arrow.up", true)
        case .error:
            if ctrl.activeEndpoint == nil {
                ("启动工作站", "power", ctrl.isBusy)
            } else if !ctrl.localTunnelReady {
                ("重连隧道", "arrow.clockwise", ctrl.isBusy)
            } else {
                ("重新检查", "arrow.clockwise", ctrl.isBusy)
            }
        }

        Button {
            switch ctrl.phase {
            case .off:
                ctrl.startWorkstation()
            case .auth:
                if ctrl.consentOpenedInBrowser {
                    ctrl.startWorkstation()
                } else {
                    ctrl.openConsentPage()
                }
            case .ready:
                ctrl.openComfyUI()
            case .error:
                if ctrl.activeEndpoint == nil {
                    ctrl.startWorkstation()
                } else if !ctrl.localTunnelReady && ctrl.errorMessage?.contains("18188") == true {
                    ctrl.reconnectTunnel()
                } else {
                    Task { await ctrl.refreshProbe() }
                }
            case .booting, .copy, .stopping:
                break
            }
        } label: {
            HStack(spacing: 7) {
                Image(systemName: icon)
                    .font(.system(size: 12.5, weight: .semibold))
                Text(title)
                    .font(.system(size: 13, weight: .semibold))
            }
            .foregroundColor(.white)
            .padding(.horizontal, 18)
            .frame(height: 40)
            .background(disabled ? Palette.accent.opacity(0.5) : Palette.accent, in: RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .disabled(disabled)
    }

    private var inventorySection: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("已保存的模型")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundColor(Palette.ink)
                Spacer()
                Text(inventoryHeaderMeta)
                    .font(.system(size: 11))
                    .foregroundColor(Palette.muted)
            }

            switch ctrl.inventoryState {
            case .loading:
                Text("正在读取云盘保存目录…")
                    .font(.system(size: 12))
                    .foregroundColor(Palette.muted)
                    .padding(.vertical, 24)
            case .uninitialized:
                Text("尚未扫描保存目录。首次开机连接云盘后，会自动记录实际保存的模型文件。")
                    .font(.system(size: 12))
                    .foregroundColor(Palette.muted)
                    .padding(.vertical, 22)
            case let .failed(msg):
                VStack(alignment: .leading, spacing: 8) {
                    Text(msg)
                        .font(.system(size: 12))
                        .foregroundColor(Palette.red)
                    Button("重试读取") {
                        Task { await ctrl.refreshProbe() }
                    }
                }
                .padding(.vertical, 16)
            case let .empty(root, cached, observedAt):
                VStack(alignment: .leading, spacing: 6) {
                    Text(root)
                        .font(.system(size: 11))
                        .foregroundColor(Palette.muted)
                    Text("保存目录中还没有模型文件。" + (cached ? "（上次读取：\(formatObservedTime(observedAt))）" : ""))
                        .font(.system(size: 12))
                        .foregroundColor(Palette.muted)
                        .padding(.vertical, 18)
                }
            case let .loaded(inv):
                let isCached = inv.cached == true || ctrl.phase == .off || ctrl.phase == .auth
                VStack(alignment: .leading, spacing: 0) {
                    Text(inv.root)
                        .font(.system(size: 11))
                        .foregroundColor(Palette.muted)
                        .padding(.bottom, 10)

                    Divider().overlay(Palette.line)

                    ScrollView {
                        VStack(spacing: 0) {
                            ForEach(inv.files) { file in
                                modelRow(file, isCached: isCached)
                                Divider().overlay(Palette.line.opacity(0.6))
                            }
                        }
                    }
                    .scrollIndicators(.never)

                    HStack {
                        Text(isCached
                             ? "上次读取：\(formatObservedTime(inv.observedAt)) · 待开机核对"
                             : "实时读取：\(formatObservedTime(inv.observedAt))")
                            .font(.system(size: 11))
                            .foregroundColor(Palette.muted)
                        Spacer()
                        if let wf = inv.workflowCount {
                            Label("\(wf) 个工作流", systemImage: "square.grid.2x2")
                                .font(.system(size: 11))
                                .foregroundColor(Palette.muted)
                        }
                    }
                    .padding(.top, 10)
                }
            }
        }
    }

    private func modelRow(_ file: ModelInventoryFile, isCached: Bool) -> some View {
        let isLocalReady = !isCached && file.status == "ready" && (file.copiedBytes ?? 0) == file.size && file.size > 0
        let statusLabel: String = if isLocalReady {
            "本地就绪"
        } else if !isCached && file.status == "copying" {
            file.source == "hf" ? "拉取中" : "拷贝中"
        } else if isCached {
            file.source == "hf" ? "开机直拉" : "待核对"
        } else if file.status == "pending" {
            file.source == "hf" ? "待拉取" : "待拷贝"
        } else {
            "未核对"
        }

        return HStack(spacing: 12) {
            RoundedRectangle(cornerRadius: 7, style: .continuous)
                .fill(Palette.sidebar)
                .frame(width: 32, height: 34)
                .overlay(RoundedRectangle(cornerRadius: 7, style: .continuous).stroke(Palette.line, lineWidth: 1))
                .overlay {
                    Text(file.shortBadge)
                        .font(.system(size: 9.5, weight: .semibold))
                        .foregroundColor(Palette.muted)
                }

            VStack(alignment: .leading, spacing: 3) {
                Text(file.filename)
                    .font(.system(size: 12, weight: .medium))
                    .foregroundColor(Palette.ink)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Text(file.category)
                    .font(.system(size: 10.5))
                    .foregroundColor(Palette.muted)
            }

            Spacer()

            Text(formatByteCount(file.size))
                .font(.system(size: 11).monospacedDigit())
                .foregroundColor(Palette.muted)
                .frame(minWidth: 68, alignment: .trailing)

            Text(statusLabel)
                .font(.system(size: 10.5, weight: isLocalReady ? .medium : .regular))
                .foregroundColor(isLocalReady ? Palette.green : Palette.muted)
                .frame(minWidth: 50, alignment: .trailing)
        }
        .padding(.vertical, 10)
    }

    private var stepsSection: some View {
        let totalText: String = {
            if let inv = ctrl.currentInventory {
                let total = inv.files.reduce(Int64(0)) { $0 + $1.size }
                return "\(inv.files.count) 个文件，共 \(formatByteCount(total))"
            }
            return "按保存目录实际大小拷贝"
        }()

        let steps = [
            ("分配 GPU", "从此时开始计费"),
            ("连接 Google Drive", "需要时会提示授权"),
            ("准备环境与模型", totalText),
            ("启动 ComfyUI", "准备好后即可打开"),
        ]

        return VStack(alignment: .leading, spacing: 14) {
            Text(ctrl.phase == .off ? "开机后自动完成" : ctrl.phase == .ready ? "启动已完成" : "启动进度")
                .font(.system(size: 13, weight: .semibold))
                .foregroundColor(Palette.ink)

            VStack(alignment: .leading, spacing: 16) {
                ForEach(Array(steps.enumerated()), id: \.offset) { idx, item in
                    let done = idx < ctrl.activeStepIndex
                    let active = idx == ctrl.activeStepIndex
                    HStack(alignment: .top, spacing: 10) {
                        ZStack {
                            Circle()
                                .fill(done ? Palette.greenBg : active ? Palette.accent : Palette.paper)
                                .frame(width: 19, height: 19)
                                .overlay {
                                    Circle().stroke(done ? Palette.green.opacity(0.35) : active ? Palette.accent : Palette.line, lineWidth: 1)
                                }
                            if done {
                                Image(systemName: "checkmark")
                                    .font(.system(size: 9.5, weight: .bold))
                                    .foregroundColor(Palette.green)
                            } else {
                                Text("\(idx + 1)")
                                    .font(.system(size: 9.5, weight: .semibold))
                                    .foregroundColor(active ? .white : Palette.muted)
                            }
                        }

                        VStack(alignment: .leading, spacing: 2) {
                            Text(item.0)
                                .font(.system(size: 12, weight: active ? .semibold : .regular))
                                .foregroundColor(active ? Palette.accent : done ? Palette.ink : Palette.muted)
                            Text(item.1)
                                .font(.system(size: 10.5))
                                .foregroundColor(Palette.muted)
                        }
                    }
                }
            }
        }
    }

    private var modelCountBadge: String? {
        switch ctrl.inventoryState {
        case .loading: return "…"
        case let .loaded(inv): return "\(inv.files.count)"
        case .empty: return "0"
        case .uninitialized, .failed: return nil
        }
    }

    private var inventoryHeaderMeta: String {
        switch ctrl.inventoryState {
        case .loading: return "读取中"
        case .uninitialized: return "未读取"
        case .failed: return "状态未知"
        case .empty: return "0 个文件"
        case let .loaded(inv):
            let total = inv.files.reduce(Int64(0)) { $0 + $1.size }
            let isCached = inv.cached == true || ctrl.phase == .off || ctrl.phase == .auth
            if isCached {
                return "\(inv.files.count) 个文件 · \(formatByteCount(total))"
            }
            let readyCount = inv.files.filter { $0.status == "ready" && ($0.copiedBytes ?? 0) == $0.size }.count
            return "\(readyCount) / \(inv.files.count) 本地就绪 · \(formatByteCount(total))"
        }
    }

    private var badgeStyle: (String, Color, Color, Color) {
        switch ctrl.phase {
        case .off:
            return ("已关机 · 不计费", Palette.muted, Palette.neutralBadge, Palette.line)
        case .booting:
            return ("开机中 · 正在计费", Palette.accent, Color(red: 0.925, green: 0.925, blue: 0.988), Palette.accent.opacity(0.25))
        case .auth:
            return ("待授权 · 正在计费", Palette.amber, Palette.amberBg, Palette.amber.opacity(0.28))
        case .copy:
            return ("准备中 · 正在计费", Palette.accent, Color(red: 0.925, green: 0.925, blue: 0.988), Palette.accent.opacity(0.25))
        case .ready:
            return ("运行中 · 正在计费", Palette.green, Palette.greenBg, Palette.green.opacity(0.25))
        case .stopping:
            return ("同步关机中", Palette.amber, Palette.amberBg, Palette.amber.opacity(0.28))
        case .error:
            return ctrl.activeEndpoint != nil
                ? ("连接异常 · 可能仍在计费", Palette.red, Palette.redBg, Palette.red.opacity(0.25))
                : ("分配未成功 · 未计费", Palette.amber, Palette.amberBg, Palette.amber.opacity(0.28))
        }
    }

    private var stateTitle: String {
        switch ctrl.phase {
        case .off: return "工作站已关机"
        case .booting:
            return switch ctrl.currentSubPhase {
            case "mounting_drive": "正在连接 Google Drive"
            case "extracting": "正在恢复 ComfyUI 运行环境"
            case "syncing_models": "正在检查云盘模型"
            default: "正在分配 GPU 实例"
            }
        case .auth: return "允许这台机器访问云盘"
        case .copy:
            return ctrl.currentSubPhase == "starting_comfy" ? "正在启动 ComfyUI" : "正在从 Hugging Face 拉取模型"
        case .ready: return "工作站已就绪"
        case .stopping: return "正在同步工作流与输出回云盘"
        case .error:
            return ctrl.activeEndpoint != nil ? "暂时无法确认机器状态" : "未能分配到 \(ctrl.selectedGPU) 实例"
        }
    }

    private var stateDescription: String {
        switch ctrl.phase {
        case .off:
            return "启动后从 Hugging Face 16 线程直拉模型到本地盘，工作流与 output 关机自动同步回云盘。"
        case .booting:
            return "完成后会自动进入模型拉取与启动，无需手动敲命令。"
        case .auth:
            return "Colab 每开新机需授权一次云盘。在浏览器选择 20x 账号完成授权，再回来点继续。"
        case .copy:
            if ctrl.currentSubPhase == "starting_comfy" {
                return "模型已就位，正在等 ComfyUI 服务监听端口。"
            }
            if let prog = ctrl.copyProgress {
                return "\(formatByteCount(prog.copied)) / \(formatByteCount(prog.total)) 已拉取到本地盘，完成后自动启动。"
            }
            return "正在从 Hugging Face 16 线程并发拉取模型到实例本地盘。"
        case .ready:
            if let inv = ctrl.currentInventory {
                let ready = inv.files.filter { $0.status == "ready" && ($0.copiedBytes ?? 0) == $0.size }.count
                return "ComfyUI 已启动；\(ready) / \(inv.files.count) 个模型已就绪，工作流与输出均运行在本地盘。"
            }
            return "ComfyUI 已启动并连上隧道。"
        case .stopping:
            return "正在把修改过的 JSON 工作流与新生成的 output 图片写回 Google Drive 并等待刷盘…"
        case .error:
            return ctrl.errorMessage ?? "请点「重新检查」确认当前实例状态，或点「关机」停止计费。"
        }
    }

    private var inlineNoteText: String {
        switch ctrl.phase {
        case .off: return "模型、工作流和已保存的图片保留在 Google Drive。"
        case .booting, .copy: return "云端正在自行准备。注意：直接关窗口不会关机，也不会停止计费。"
        case .auth: return "GPU 已分配并开始计费。若暂时不用，可随时点「关机」。"
        case .ready:
            let rate = ctrl.burnRateHourly > 0 ? String(format: "当前约 %.2f 点 / 小时。", ctrl.burnRateHourly) : ""
            return "\(rate)用完请点「关机」，会自动把工作流 JSON 和 output 同步回云盘。"
        case .stopping: return "刷盘完成后会自动删除实例并停止计费，请勿强退。"
        case .error:
            return ctrl.activeEndpoint != nil
                ? "网络断开不代表关机，若实例仍在运行会继续计费。"
                : "本次未能成功分配实例，没有产生计费。"
        }
    }

    private var footerStatusText: String {
        switch ctrl.phase {
        case .off: return "启动后可用"
        case .booting, .copy: return "准备完成后可用"
        case .auth: return "等待完成云盘授权"
        case .ready: return ctrl.localTunnelReady ? "局域网入口已连接" : "隧道未连接"
        case .stopping: return "正在同步回云盘…"
        case .error: return ctrl.activeEndpoint != nil ? "入口暂不可用" : "未分配实例 · 不计费"
        }
    }

    private var logPageSubtitle: String {
        guard let ep = ctrl.activeEndpoint else { return "当前无活跃实例" }
        if let reg = ctrl.activeRegionLabel, !reg.isEmpty {
            return "当前实例：\(ep) · \(reg)"
        }
        return "当前实例：\(ep)"
    }

    private var logSummaryText: String {
        ctrl.logText.split(separator: "\n").last.map(String.init) ?? "无日志"
    }
}
