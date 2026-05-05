use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receiver_accepts_private_announces_from_multiple_configured_networks() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_a = "nostr-vpn-multi-network-a".to_string();
    let network_b = "nostr-vpn-multi-network-b".to_string();

    let sender_a_keys = Keys::generate();
    let sender_b_keys = Keys::generate();
    let receiver_keys = Keys::generate();
    let sender_a_pubkey = sender_a_keys.public_key().to_hex();
    let sender_b_pubkey = sender_b_keys.public_key().to_hex();
    let receiver_pubkey = receiver_keys.public_key().to_hex();

    let sender_a = NostrSignalingClient::new_with_keys(
        network_a.clone(),
        sender_a_keys,
        vec![sender_a_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("sender a client");
    let sender_b = NostrSignalingClient::new_with_keys(
        network_b.clone(),
        sender_b_keys,
        vec![sender_b_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("sender b client");
    let receiver = NostrSignalingClient::new_with_keys_and_networks(
        receiver_keys,
        vec![
            SignalingNetwork {
                network_id: network_a.clone(),
                participants: vec![sender_a_pubkey.clone(), receiver_pubkey.clone()],
            },
            SignalingNetwork {
                network_id: network_b.clone(),
                participants: vec![sender_b_pubkey.clone(), receiver_pubkey.clone()],
            },
        ],
    )
    .expect("receiver client");

    sender_a
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender a connect");
    sender_b
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender b connect");
    receiver
        .connect(&[relay_url])
        .await
        .expect("receiver connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let announcement_a = PeerAnnouncement {
        node_id: "sender-a-node".to_string(),
        public_key: "sender-a-public".to_string(),
        endpoint: "127.0.0.1:51820".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.0.11/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 10,
    };
    let announcement_b = PeerAnnouncement {
        node_id: "sender-b-node".to_string(),
        public_key: "sender-b-public".to_string(),
        endpoint: "127.0.0.1:51821".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.0.12/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 11,
    };

    sender_a
        .publish(SignalPayload::Announce(announcement_a.clone()))
        .await
        .expect("sender a publish should succeed");
    sender_b
        .publish(SignalPayload::Announce(announcement_b.clone()))
        .await
        .expect("sender b publish should succeed");

    let mut received = BTreeMap::new();
    while received.len() < 2 {
        let message = timeout(Duration::from_secs(5), receiver.recv())
            .await
            .expect("timed out waiting for multi-network announce")
            .expect("message expected");
        received.insert(message.network_id, message.payload);
    }

    assert_eq!(
        received.get(&network_a),
        Some(&SignalPayload::Announce(announcement_a))
    );
    assert_eq!(
        received.get(&network_b),
        Some(&SignalPayload::Announce(announcement_b))
    );

    sender_a.disconnect().await;
    sender_b.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receiver_rejects_private_announce_from_sender_outside_target_network() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_a = "nostr-vpn-multi-membership-a".to_string();
    let network_b = "nostr-vpn-multi-membership-b".to_string();

    let sender_keys = Keys::generate();
    let receiver_keys = Keys::generate();
    let legitimate_network_b_keys = Keys::generate();
    let sender_pubkey = sender_keys.public_key().to_hex();
    let receiver_pubkey = receiver_keys.public_key().to_hex();
    let legitimate_network_b_pubkey = legitimate_network_b_keys.public_key().to_hex();

    let sender = NostrSignalingClient::new_with_keys(
        network_b,
        sender_keys,
        vec![sender_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("sender client");
    let receiver = NostrSignalingClient::new_with_keys_and_networks(
        receiver_keys,
        vec![
            SignalingNetwork {
                network_id: network_a,
                participants: vec![sender_pubkey.clone(), receiver_pubkey.clone()],
            },
            SignalingNetwork {
                network_id: "nostr-vpn-multi-membership-b".to_string(),
                participants: vec![legitimate_network_b_pubkey.clone(), receiver_pubkey.clone()],
            },
        ],
    )
    .expect("receiver client");

    sender
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender connect");
    receiver
        .connect(&[relay_url])
        .await
        .expect("receiver connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    let spoofed_announcement = PeerAnnouncement {
        node_id: "spoofed-node".to_string(),
        public_key: "spoofed-public".to_string(),
        endpoint: "127.0.0.1:51822".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.0.13/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 12,
    };

    sender
        .publish(SignalPayload::Announce(spoofed_announcement))
        .await
        .expect("sender publish should succeed");

    let missing = timeout(Duration::from_millis(750), receiver.recv()).await;
    assert!(
        missing.is_err(),
        "receiver should ignore private announce from a sender not configured for that network"
    );

    sender.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_is_labeled_with_the_matching_network_in_multi_network_mode() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_a = "nostr-vpn-multi-hello-a".to_string();
    let network_b = "nostr-vpn-multi-hello-b".to_string();

    let sender_a_keys = Keys::generate();
    let sender_b_keys = Keys::generate();
    let receiver_keys = Keys::generate();
    let sender_a_pubkey = sender_a_keys.public_key().to_hex();
    let sender_b_pubkey = sender_b_keys.public_key().to_hex();
    let receiver_pubkey = receiver_keys.public_key().to_hex();

    let sender_b = NostrSignalingClient::new_with_keys(
        network_b.clone(),
        sender_b_keys,
        vec![sender_b_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("sender b client");
    let receiver = NostrSignalingClient::new_with_keys_and_networks(
        receiver_keys,
        vec![
            SignalingNetwork {
                network_id: network_a,
                participants: vec![sender_a_pubkey, receiver_pubkey.clone()],
            },
            SignalingNetwork {
                network_id: network_b.clone(),
                participants: vec![sender_b_pubkey.clone(), receiver_pubkey],
            },
        ],
    )
    .expect("receiver client");

    sender_b
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender b connect");
    receiver
        .connect(&[relay_url])
        .await
        .expect("receiver connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    sender_b
        .publish(SignalPayload::Hello)
        .await
        .expect("hello publish should succeed");

    let received = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("timed out waiting for hello")
        .expect("message expected");

    assert_eq!(received.network_id, network_b);
    assert_eq!(received.sender_pubkey, sender_b_pubkey);
    assert_eq!(received.payload, SignalPayload::Hello);

    sender_b.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}
