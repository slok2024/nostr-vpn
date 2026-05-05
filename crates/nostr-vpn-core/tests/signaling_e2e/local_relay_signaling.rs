use super::*;

#[test]
fn signaling_kind_uses_hashtree_style_ephemeral_range() {
    assert_eq!(NOSTR_KIND_NOSTR_VPN, 25_050);
    assert!((20_000..30_000).contains(&NOSTR_KIND_NOSTR_VPN));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn announces_over_local_nostr_relay() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-test".to_string();

    let sender_keys = Keys::generate();
    let receiver_keys = Keys::generate();
    let sender_pubkey = sender_keys.public_key().to_hex();
    let receiver_pubkey = receiver_keys.public_key().to_hex();

    let sender = NostrSignalingClient::new_with_keys(
        network_id.clone(),
        sender_keys,
        vec![sender_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("sender client");
    let receiver = NostrSignalingClient::new_with_keys(
        network_id,
        receiver_keys,
        vec![sender_pubkey, receiver_pubkey],
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

    let announcement = PeerAnnouncement {
        node_id: "sender-node".to_string(),
        public_key: "sender-public".to_string(),
        endpoint: "127.0.0.1:51820".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.0.5/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 42,
    };

    sender
        .publish(SignalPayload::Announce(announcement.clone()))
        .await
        .expect("publish should succeed");

    let received = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("timed out waiting for message")
        .expect("message expected");

    assert_eq!(received.network_id, "nostr-vpn-test");
    assert_eq!(received.payload, SignalPayload::Announce(announcement));

    sender.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_presence_is_received_over_local_nostr_relay() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-test-hello".to_string();

    let sender_keys = Keys::generate();
    let receiver_keys = Keys::generate();
    let sender_pubkey = sender_keys.public_key().to_hex();
    let receiver_pubkey = receiver_keys.public_key().to_hex();

    let sender = NostrSignalingClient::new_with_keys(
        network_id.clone(),
        sender_keys,
        vec![sender_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("sender client");
    let receiver = NostrSignalingClient::new_with_keys(
        network_id,
        receiver_keys,
        vec![sender_pubkey.clone(), receiver_pubkey],
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

    sender
        .publish(SignalPayload::Hello)
        .await
        .expect("hello publish should succeed");

    let received = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("timed out waiting for hello")
        .expect("message expected");

    assert_eq!(received.payload, SignalPayload::Hello);

    sender.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn join_requests_are_received_on_the_normal_signaling_connection() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let owner_keys = Keys::generate();
    let requester_keys = Keys::generate();
    let owner_pubkey = owner_keys.public_key().to_hex();
    let requester_pubkey = requester_keys.public_key().to_hex();

    let receiver = NostrSignalingClient::new_with_keys(
        "mesh-home".to_string(),
        owner_keys,
        vec![requester_pubkey.clone()],
    )
    .expect("receiver client");
    receiver
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("receiver connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    publish_join_request(
        requester_keys,
        std::slice::from_ref(&relay_url),
        owner_pubkey,
        MeshJoinRequest {
            network_id: "mesh-home".to_string(),
            requester_node_name: "alice-phone".to_string(),
        },
    )
    .await
    .expect("join request publish");

    let received = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("timed out waiting for join request")
        .expect("message expected");

    assert_eq!(received.network_id, "mesh-home");
    assert_eq!(received.sender_pubkey, requester_pubkey);
    match received.payload {
        SignalPayload::JoinRequest {
            requested_at,
            request,
        } => {
            assert!(requested_at > 0);
            assert_eq!(
                request,
                MeshJoinRequest {
                    network_id: "mesh-home".to_string(),
                    requester_node_name: "alice-phone".to_string(),
                }
            );
        }
        other => panic!("expected join request payload, got {other:?}"),
    }

    receiver.disconnect().await;
    relay.stop().await;
}
