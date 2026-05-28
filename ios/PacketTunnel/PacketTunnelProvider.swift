import Foundation
import NetworkExtension
import Darwin

private let appGroupIdentifier = "group.to.iris.nvpn"

final class PacketTunnelProvider: NEPacketTunnelProvider {
    private static let nextPacketPollTimeoutMs: UInt32 = 100

    private var tunnelHandle: OpaquePointer?
    private var tunnelRunning = false
    private var activeTunnelCalls = 0
    private let tunnelCondition = NSCondition()
    private let packetQueue = DispatchQueue(label: "to.iris.nvpn.packet-tunnel", qos: .userInitiated)

    override func startTunnel(
        options: [String: NSObject]?,
        completionHandler: @escaping (Error?) -> Void
    ) {
        NSLog("nvpn-pkt: startTunnel entered")
        packetDebugLog("startTunnel entered options=\(options.map { Array($0.keys).sorted() } ?? [])")
        let configuration = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration ?? [:]
        packetDebugLog("providerConfiguration keys=\(Array(configuration.keys).sorted())")
        let optionConfigJson = options?["mobileTunnelConfigJson"] as? String
        let configJson = optionConfigJson ?? configuration["mobileTunnelConfigJson"] as? String ?? ""
        let parsedConfig = MobileTunnelConfig(json: configJson)
        if let error = parsedConfig.errorText {
            NSLog("nvpn-pkt: config parse failed: \(error)")
            packetDebugLog("config parse failed: \(error)")
            completionHandler(error)
            return
        }
        NSLog("nvpn-pkt: calling nostr_vpn_mobile_tunnel_new (configLen=\(configJson.count))")
        packetDebugLog("calling nostr_vpn_mobile_tunnel_new configLen=\(configJson.count)")
        guard let handle = configJson.withCString({ nostr_vpn_mobile_tunnel_new($0) }) else {
            NSLog("nvpn-pkt: nostr_vpn_mobile_tunnel_new returned NULL")
            packetDebugLog("nostr_vpn_mobile_tunnel_new returned NULL")
            completionHandler(PacketTunnelError.startFailed)
            return
        }
        NSLog("nvpn-pkt: rust runtime up, handle=\(handle)")
        packetDebugLog("rust runtime up")
        tunnelCondition.lock()
        tunnelHandle = handle
        tunnelRunning = true
        activeTunnelCalls = 0
        tunnelCondition.unlock()
        var excludedRoutes = parsedConfig.excludedRoutes
        let resolvedWgExcludedRoute = consumeCString(
            nostr_vpn_mobile_tunnel_wg_excluded_route(handle)
        ).trimmingCharacters(in: .whitespacesAndNewlines)
        if !resolvedWgExcludedRoute.isEmpty && !excludedRoutes.contains(resolvedWgExcludedRoute) {
            excludedRoutes.append(resolvedWgExcludedRoute)
            packetDebugLog("added resolved WG excluded route \(resolvedWgExcludedRoute)")
        }

        // tunnelRemoteAddress is what iOS shows in Settings → VPN
        // and uses to decide "where the tunnel goes". wireguard-apple
        // points it at the actual WG endpoint host, not TEST-NET. iOS
        // will refuse to flip the status badge to "connected"+icon if
        // it deems the remote address bogus.
        let remoteAddress = parsedConfig.firstWireGuardEndpointHost ?? "10.0.0.1"
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: remoteAddress)
        settings.mtu = NSNumber(value: parsedConfig.mtu)

        if let parsed = parseIPv4CIDR(parsedConfig.localAddress) {
            let ipv4 = NEIPv4Settings(addresses: [parsed.address], subnetMasks: [parsed.mask])
            // Use NEIPv4Route.default() for 0.0.0.0/0 — iOS recognizes
            // it as the catch-all default route, vs an explicit
            // (0.0.0.0, 0.0.0.0) which can be treated as a host route
            // in some kernel paths.
            ipv4.includedRoutes = parsedConfig.routeTargets.compactMap(ipv4Route)
            // When WG upstream is on, the Rust runtime has expanded
            // `routeTargets` to include 0.0.0.0/0 so all outbound
            // traffic enters the tun. We then exclude the resolved WG
            // endpoint IP itself so the encrypted UDP can actually
            // escape the tunnel and reach the upstream.
            if !excludedRoutes.isEmpty {
                ipv4.excludedRoutes = excludedRoutes.compactMap(ipv4Route)
            }
            settings.ipv4Settings = ipv4
            NSLog(
                "nvpn-pkt: ipv4 addr=\(parsed.address)/\(parsed.mask) "
                    + "included=\(parsedConfig.routeTargets) "
                    + "excluded=\(excludedRoutes)"
            )
        }

        // DNS resolvers — Mullvad/Proton ship their own (e.g.
        // 10.64.0.1) which lives behind the tunnel. Without
        // `dnsSettings` here, iOS falls back to whatever the
        // underlying Wi-Fi provided — which doesn't help once
        // 0.0.0.0/0 is on the tun, because every DNS query goes into
        // utun and toward Mullvad's WG endpoint, which doesn't run a
        // resolver. The Rust side falls back to public resolvers when
        // the user's config didn't include DNS.
        if !parsedConfig.dnsServers.isEmpty {
            let dns = NEDNSSettings(servers: parsedConfig.dnsServers)
            dns.matchDomains = [""] // catch-all so VPN DNS handles every query
            settings.dnsSettings = dns
            NSLog("nvpn-pkt: dns servers=\(parsedConfig.dnsServers)")
        }

        NSLog("nvpn-pkt: calling setTunnelNetworkSettings")
        packetDebugLog("calling setTunnelNetworkSettings")
        setTunnelNetworkSettings(settings) { [weak self] error in
            if let error {
                NSLog("nvpn-pkt: setTunnelNetworkSettings failed: \(error)")
                packetDebugLog("setTunnelNetworkSettings failed: \(error)")
                self?.stopRustTunnel()
                completionHandler(error)
                return
            }
            NSLog("nvpn-pkt: setTunnelNetworkSettings succeeded — starting packet loops")
            packetDebugLog("setTunnelNetworkSettings succeeded")
            self?.startPacketLoops()
            NSLog("nvpn-pkt: completionHandler(nil) — VPN should transition to connected")
            packetDebugLog("completionHandler nil")
            completionHandler(nil)
        }
    }

    override func stopTunnel(
        with reason: NEProviderStopReason,
        completionHandler: @escaping () -> Void
    ) {
        packetDebugLog("stopTunnel reason=\(reason.rawValue)")
        stopRustTunnel()
        completionHandler()
    }

    override func handleAppMessage(
        _ messageData: Data,
        completionHandler: ((Data?) -> Void)?
    ) {
        let message = String(data: messageData, encoding: .utf8) ?? ""
        switch message {
        case "runtimeState":
            let json = withTunnelHandle { handle in
                consumeCString(nostr_vpn_mobile_tunnel_runtime_state_json(handle))
            } ?? #"{"error":"mobile tunnel stopped"}"#
            completionHandler?(json.data(using: .utf8))
        case "takeAppConfig":
            let toml = withTunnelHandle { handle in
                consumeCString(nostr_vpn_mobile_tunnel_take_app_config_toml(handle))
            } ?? ""
            completionHandler?(toml.data(using: .utf8))
        default:
            completionHandler?(nil)
        }
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
                    return nostr_vpn_mobile_tunnel_next_packet(
                        handle,
                        base,
                        UInt(capacity),
                        Self.nextPacketPollTimeoutMs
                    )
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
        tunnelCondition.lock()
        tunnelRunning = false
        let handle = tunnelHandle
        tunnelHandle = nil
        while activeTunnelCalls > 0 {
            tunnelCondition.wait()
        }
        tunnelCondition.unlock()

        if let handle {
            nostr_vpn_mobile_tunnel_free(handle)
        }
    }

    private func isTunnelRunning() -> Bool {
        tunnelCondition.lock()
        defer { tunnelCondition.unlock() }
        return tunnelRunning
    }

    private func withTunnelHandle<T>(_ body: (OpaquePointer) -> T) -> T? {
        tunnelCondition.lock()
        guard tunnelRunning, let handle = tunnelHandle else {
            tunnelCondition.unlock()
            return nil
        }
        activeTunnelCalls += 1
        tunnelCondition.unlock()

        defer {
            tunnelCondition.lock()
            activeTunnelCalls -= 1
            if activeTunnelCalls == 0 {
                tunnelCondition.broadcast()
            }
            tunnelCondition.unlock()
        }

        return body(handle)
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
    let excludedRoutes: [String]
    let dnsServers: [String]
    let firstWireGuardEndpointHost: String?
    let mtu: Int
    let errorText: Error?

    init(json: String) {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            localAddress = "10.44.0.1/32"
            routeTargets = []
            excludedRoutes = []
            dnsServers = []
            firstWireGuardEndpointHost = nil
            mtu = 1280
            errorText = PacketTunnelError.invalidConfig("Invalid tunnel configuration")
            return
        }
        let error = object["error"] as? String ?? ""
        localAddress = object["localAddress"] as? String ?? "10.44.0.1/32"
        routeTargets = object["routeTargets"] as? [String] ?? []
        excludedRoutes = object["excludedRoutes"] as? [String] ?? []
        dnsServers = object["dnsServers"] as? [String] ?? []
        if let wg = object["wireguardExit"] as? [String: Any],
           let endpoint = wg["endpoint"] as? String
        {
            // Endpoint is host:port — strip the port for tunnelRemoteAddress.
            firstWireGuardEndpointHost = endpoint.split(separator: ":", maxSplits: 1).first.map(String.init)
        } else {
            firstWireGuardEndpointHost = nil
        }
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
    if parsed.address == "0.0.0.0" && parsed.mask == "0.0.0.0" {
        // iOS treats `NEIPv4Route.default()` and an explicit
        // (0.0.0.0/0) differently in some kernel paths — the former
        // is the documented default route. Always normalize.
        return NEIPv4Route.default()
    }
    return NEIPv4Route(destinationAddress: parsed.address, subnetMask: parsed.mask)
}

private func packetFamily(_ packet: Data) -> NSNumber {
    guard let first = packet.first else {
        return NSNumber(value: AF_INET)
    }
    return NSNumber(value: (first >> 4) == 6 ? AF_INET6 : AF_INET)
}

private func consumeCString(_ pointer: UnsafeMutablePointer<CChar>?) -> String {
    guard let pointer else {
        return ""
    }
    defer { nostr_vpn_string_free(pointer) }
    return String(cString: pointer)
}

private func packetDebugLog(_ message: String) {
    #if DEBUG
    let logDir = FileManager.default
        .containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier)?
        .appendingPathComponent("Nostr VPN", isDirectory: true)
        ?? FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)
            .first
        ?? FileManager.default.temporaryDirectory
    try? FileManager.default.createDirectory(at: logDir, withIntermediateDirectories: true)
    let logUrl = logDir.appendingPathComponent("nvpn-pkt-debug.log")
    let line = "[\(Date())] \(message)\n"
    guard let data = line.data(using: .utf8) else {
        return
    }
    if FileManager.default.fileExists(atPath: logUrl.path),
       let handle = try? FileHandle(forWritingTo: logUrl)
    {
        handle.seekToEndOfFile()
        handle.write(data)
        try? handle.close()
    } else {
        try? data.write(to: logUrl)
    }
    #endif
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
