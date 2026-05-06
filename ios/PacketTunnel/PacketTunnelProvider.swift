import Foundation
import NetworkExtension
import Darwin

final class PacketTunnelProvider: NEPacketTunnelProvider {
    private var tunnelHandle: OpaquePointer?
    private var tunnelRunning = false
    private let tunnelLock = NSLock()
    private let packetQueue = DispatchQueue(label: "to.iris.nvpn.packet-tunnel", qos: .userInitiated)

    override func startTunnel(
        options: [String: NSObject]?,
        completionHandler: @escaping (Error?) -> Void
    ) {
        let configuration = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration ?? [:]
        let configJson = configuration["mobileTunnelConfigJson"] as? String ?? ""
        let parsedConfig = MobileTunnelConfig(json: configJson)
        if let error = parsedConfig.errorText {
            completionHandler(error)
            return
        }
        guard let handle = configJson.withCString({ nostr_vpn_mobile_tunnel_new($0) }) else {
            completionHandler(PacketTunnelError.startFailed)
            return
        }
        tunnelLock.lock()
        tunnelHandle = handle
        tunnelRunning = true
        tunnelLock.unlock()

        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "192.0.2.1")
        settings.mtu = NSNumber(value: parsedConfig.mtu)

        if let parsed = parseIPv4CIDR(parsedConfig.localAddress) {
            let ipv4 = NEIPv4Settings(addresses: [parsed.address], subnetMasks: [parsed.mask])
            ipv4.includedRoutes = parsedConfig.routeTargets.compactMap(ipv4Route)
            settings.ipv4Settings = ipv4
        }

        setTunnelNetworkSettings(settings) { [weak self] error in
            if let error {
                self?.stopRustTunnel()
                completionHandler(error)
                return
            }
            self?.startPacketLoops()
            completionHandler(nil)
        }
    }

    override func stopTunnel(
        with reason: NEProviderStopReason,
        completionHandler: @escaping () -> Void
    ) {
        stopRustTunnel()
        completionHandler()
    }

    private func startPacketLoops() {
        readPackets()
        packetQueue.async { [weak self] in
            self?.writePackets()
        }
    }

    private func readPackets() {
        guard isTunnelRunning() else {
            return
        }
        packetFlow.readPackets { [weak self] packets, _ in
            guard let self else {
                return
            }
            guard self.isTunnelRunning() else {
                return
            }
            for packet in packets {
                packet.withUnsafeBytes { raw in
                    guard let base = raw.bindMemory(to: UInt8.self).baseAddress else {
                        return
                    }
                    _ = self.withTunnelHandle { handle in
                        nostr_vpn_mobile_tunnel_send_packet(handle, base, UInt(packet.count))
                    }
                }
            }
            self.readPackets()
        }
    }

    private func writePackets() {
        var buffer = [UInt8](repeating: 0, count: 65_535)
        while true {
            let capacity = buffer.count
            let count = withTunnelHandle { handle -> Int in
                buffer.withUnsafeMutableBytes { raw -> Int in
                    guard let base = raw.bindMemory(to: UInt8.self).baseAddress else {
                        return -1
                    }
                    return nostr_vpn_mobile_tunnel_next_packet(handle, base, UInt(capacity), 1_000)
                }
            }
            guard let count else {
                break
            }
            if count > 0 {
                let packet = Data(buffer.prefix(count))
                let family = packetFamily(packet)
                packetFlow.writePackets([packet], withProtocols: [family])
            } else if count < 0 {
                break
            }
        }
    }

    private func stopRustTunnel() {
        tunnelLock.lock()
        tunnelRunning = false
        let handle = tunnelHandle
        tunnelHandle = nil
        tunnelLock.unlock()

        if let handle {
            nostr_vpn_mobile_tunnel_free(handle)
        }
    }

    private func isTunnelRunning() -> Bool {
        tunnelLock.lock()
        defer { tunnelLock.unlock() }
        return tunnelRunning
    }

    private func withTunnelHandle<T>(_ body: (OpaquePointer) -> T) -> T? {
        tunnelLock.lock()
        defer { tunnelLock.unlock() }
        guard tunnelRunning, let tunnelHandle else {
            return nil
        }
        return body(tunnelHandle)
    }
}

private enum PacketTunnelError: LocalizedError {
    case startFailed
    case invalidConfig(String)

    var errorDescription: String? {
        switch self {
        case .startFailed:
            return "Failed to start FIPS tunnel"
        case .invalidConfig(let message):
            return message
        }
    }
}

private struct MobileTunnelConfig {
    let localAddress: String
    let routeTargets: [String]
    let mtu: Int
    let errorText: Error?

    init(json: String) {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            localAddress = "10.44.0.1/32"
            routeTargets = []
            mtu = 1280
            errorText = PacketTunnelError.invalidConfig("Invalid tunnel configuration")
            return
        }
        let error = object["error"] as? String ?? ""
        localAddress = object["localAddress"] as? String ?? "10.44.0.1/32"
        routeTargets = object["routeTargets"] as? [String] ?? []
        mtu = object["mtu"] as? Int ?? 1280
        errorText = error.isEmpty ? nil : PacketTunnelError.invalidConfig(error)
    }
}

private func parseIPv4CIDR(_ value: String) -> (address: String, mask: String)? {
    let parts = value.split(separator: "/", maxSplits: 1, omittingEmptySubsequences: false)
    guard let address = parts.first.map(String.init), !address.isEmpty else {
        return nil
    }
    let prefix = parts.count == 2 ? Int(parts[1]) ?? 32 : 32
    guard (0...32).contains(prefix) else {
        return nil
    }
    return (address, ipv4Mask(prefixLength: prefix))
}

private func ipv4Route(_ value: String) -> NEIPv4Route? {
    guard let parsed = parseIPv4CIDR(value) else {
        return nil
    }
    return NEIPv4Route(destinationAddress: parsed.address, subnetMask: parsed.mask)
}

private func packetFamily(_ packet: Data) -> NSNumber {
    guard let first = packet.first else {
        return NSNumber(value: AF_INET)
    }
    return NSNumber(value: (first >> 4) == 6 ? AF_INET6 : AF_INET)
}

private func ipv4Mask(prefixLength: Int) -> String {
    guard prefixLength > 0 else {
        return "0.0.0.0"
    }
    let value = prefixLength == 32 ? UInt32.max : UInt32.max << UInt32(32 - prefixLength)
    return [
        String((value >> 24) & 0xff),
        String((value >> 16) & 0xff),
        String((value >> 8) & 0xff),
        String(value & 0xff),
    ].joined(separator: ".")
}
