import AppKit
import Foundation

struct ModelInventoryFile: Codable, Identifiable, Equatable {
    var id: String { path }
    let path: String
    let size: Int64
    let status: String?
    let copiedBytes: Int64?
    let source: String?

    var filename: String {
        (path as NSString).lastPathComponent
    }

    var category: String {
        let dir = (path as NSString).deletingLastPathComponent
        let base = dir.isEmpty ? "models" : dir
        if source == "hf" { return "\(base) · Hugging Face" }
        if source == "drive" { return "\(base) · Google Drive" }
        return base
    }

    var shortBadge: String {
        (path as NSString).deletingLastPathComponent == "upscale_models" ? "放大" : "模型"
    }
}

struct ModelInventory: Codable, Equatable {
    let root: String
    let observedAt: String?
    let workflowCount: Int?
    let files: [ModelInventoryFile]
    let cached: Bool?
}

struct ActiveRuntime: Codable, Equatable {
    let endpoint: String
    let accelerator: String?
    let proxyUrl: String?
    let region: String?
    let regionLabel: String?
    let tokenExpiresInSeconds: Int?
}

struct BootstrapStatusPayload: Codable, Equatable {
    let status: String?
    let phase: String?
    let error: String?
    let cache: String?
}

struct RemoteProbePayload: Codable, Equatable {
    let driveMounted: Bool?
    let bootstrap: BootstrapStatusPayload?
    let inventory: ModelInventory?
    let modelsLocal: Bool?
    let logTail: String?
}

struct LiveStatePayload: Codable, Equatable {
    let phase: String?
    let accelerator: String?
    let endpoint: String?
    let regionLabel: String?
    let driveConsentUrl: String?
    let error: String?
    let bootstrap: BootstrapStatusPayload?
    let inventory: ModelInventory?
    let logTail: String?
    let updatedAt: Int?
}

struct ProbeResponse: Codable {
    let balance: Double?
    let burnRateHourly: Double?
    let eligibleGpus: [String]?
    let activeInstances: [ActiveRuntime]?
    let localTunnel18188: Bool?
    let lanUrl: String?
    let accountConnected: Bool?
    let liveState: LiveStatePayload?
    let remote: RemoteProbePayload?
    let inventory: ModelInventory?
}

struct UpResponse: Codable {
    struct DriveInfo: Codable {
        let mounted: Bool?
        let needs_drive_consent: Bool?
        let drive_consent_url: String?
    }
    struct TunnelInfo: Codable {
        let listening: Bool?
        let url: String?
        let lan_url: String?
    }
    let ready: Bool?
    let step: String?
    let runtime: ActiveRuntime?
    let drive: DriveInfo?
    let tunnel: TunnelInfo?
}

struct ManifestSummary: Codable {
    struct ComfyInfo: Codable { let version: String? }
    struct PluginInfo: Codable { let disabled: Bool? }
    let comfyui: ComfyInfo?
    let plugins: [PluginInfo]?
}

enum WorkstationPhase: Equatable {
    case off
    case booting
    case auth
    case copy
    case ready
    case stopping
    case error
}

enum InventoryLoadState: Equatable {
    case loading
    case loaded(ModelInventory)
    case empty(String, cached: Bool, observedAt: String?)
    case uninitialized
    case failed(String)
}

enum NavSection: String, CaseIterable, Identifiable {
    case workstation = "云端工作站"
    case models = "模型与工作流"
    case logs = "运行记录"
    var id: String { rawValue }
}

func formatByteCount(_ bytes: Int64) -> String {
    if bytes < 1024 { return "\(bytes) B" }
    let units = ["KiB", "MiB", "GiB", "TiB"]
    var value = Double(bytes) / 1024.0
    var idx = 0
    while value >= 1024.0 && idx < units.count - 1 {
        value /= 1024.0
        idx += 1
    }
    return String(format: "%.2f %@", value, units[idx])
}

func formatObservedTime(_ iso: String?) -> String {
    guard let iso, !iso.isEmpty else { return "时间未记录" }
    let parser = ISO8601DateFormatter()
    parser.formatOptions = [.withInternetDateTime]
    guard let date = parser.date(from: iso) else { return iso }
    let fmt = DateFormatter()
    fmt.dateFormat = "MM-dd HH:mm:ss"
    return fmt.string(from: date)
}
