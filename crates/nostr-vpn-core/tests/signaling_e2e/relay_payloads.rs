use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publish_requires_configured_participants_for_private_signaling() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-test-private-only".to_string();
    let client = NostrSignalingClient::new(network_id).expect("client");
    client.connect(&[relay_url]).await.expect("client connect");

    let announcement = PeerAnnouncement {
        node_id: "node".to_string(),
        public_key: "pub".to_string(),
        endpoint: "127.0.0.1:51820".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.0.9/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 1,
    };

    let error = client
        .publish(SignalPayload::Announce(announcement))
        .await
        .expect_err("publish without participants should fail");
    assert!(
        error
            .to_string()
            .contains("no configured participants to send private signaling message to")
    );

    client.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_event_does_not_leak_plaintext_sensitive_fields() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-sensitive-check".to_string();
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
        network_id.clone(),
        receiver_keys,
        vec![sender_pubkey.clone(), receiver_pubkey.clone()],
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
        node_id: "node-sensitive".to_string(),
        public_key: "wg-sensitive-public".to_string(),
        endpoint: "203.0.113.77:51820".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.66.7/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 123456,
    };

    sender
        .publish(SignalPayload::Announce(announcement))
        .await
        .expect("publish should succeed");

    timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("timed out waiting for receiver")
        .expect("message expected");

    let mut relay_event = None;
    for _ in 0..50 {
        let events = relay.events_snapshot().await;
        relay_event = events
            .into_iter()
            .find(|event| event.kind == u32::from(NOSTR_KIND_NOSTR_VPN));
        if relay_event.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let event = relay_event.expect("nostr-vpn event should be stored on relay");

    assert_eq!(event.pubkey, sender_pubkey);
    assert!(
        !event.content.contains("203.0.113.77:51820"),
        "raw relay content leaked endpoint"
    );
    assert!(
        !event.content.contains("10.44.66.7/32"),
        "raw relay content leaked tunnel ip"
    );
    assert!(
        !event.content.contains("node-sensitive"),
        "raw relay content leaked node_id"
    );
    assert!(
        !event.content.contains("wg-sensitive-public"),
        "raw relay content leaked wireguard public key"
    );
    assert!(
        !event.content.contains(&network_id),
        "raw relay content leaked network_id"
    );
    assert!(
        !event.content.contains(&sender_pubkey),
        "raw relay content leaked sender pubkey envelope field"
    );
    assert!(
        event
            .tags
            .iter()
            .any(|tag| tag.len() >= 2 && tag[0] == "p" && tag[1] == receiver_pubkey),
        "event should include recipient p-tag"
    );
    assert!(
        !event
            .tags
            .iter()
            .flat_map(|tag| tag.iter())
            .any(|value| value.contains("203.0.113.77:51820") || value.contains("10.44.66.7/32")),
        "event tags leaked endpoint or tunnel ip"
    );

    sender.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hello_and_private_events_include_stable_identifiers() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-identifier-tags".to_string();
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

    sender
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    sender
        .publish(SignalPayload::Hello)
        .await
        .expect("hello publish should succeed");

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
        .publish_to(
            SignalPayload::Announce(announcement),
            std::slice::from_ref(&receiver_pubkey),
        )
        .await
        .expect("private publish should succeed");

    let mut events = Vec::new();
    for _ in 0..50 {
        events = relay.events_snapshot().await;
        if events
            .iter()
            .filter(|event| event.kind == u32::from(NOSTR_KIND_NOSTR_VPN))
            .count()
            >= 2
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let vpn_events = events
        .into_iter()
        .filter(|event| event.kind == u32::from(NOSTR_KIND_NOSTR_VPN))
        .collect::<Vec<_>>();
    assert_eq!(
        vpn_events.len(),
        2,
        "expected hello and private events on relay"
    );

    let hello_event = vpn_events
        .iter()
        .find(|event| {
            event
                .tags
                .iter()
                .any(|tag| tag.len() >= 2 && tag[0] == "l" && tag[1] == "hello")
        })
        .expect("hello event should be stored");
    assert!(
        hello_event
            .tags
            .iter()
            .any(|tag| tag.len() >= 2 && tag[0] == "d" && tag[1] == "hello"),
        "hello event should include a stable d tag",
    );

    let private_event = vpn_events
        .iter()
        .find(|event| {
            event
                .tags
                .iter()
                .any(|tag| tag.len() >= 2 && tag[0] == "p" && tag[1] == receiver_pubkey)
        })
        .expect("private event should be stored");
    assert!(
        private_event.tags.iter().any(|tag| {
            tag.len() >= 2
                && tag[0] == "d"
                && tag[1] == format!("private:{network_id}:{receiver_pubkey}")
        }),
        "private event should include a recipient-scoped d tag",
    );

    sender.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn publishes_only_current_signal_kind() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-rollout-kinds".to_string();
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

    sender
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    sender
        .publish(SignalPayload::Hello)
        .await
        .expect("hello publish should succeed");

    sender
        .publish_to(
            SignalPayload::Announce(PeerAnnouncement {
                node_id: "sender-node".to_string(),
                public_key: "sender-public".to_string(),
                endpoint: "127.0.0.1:51820".to_string(),
                local_endpoint: None,
                public_endpoint: None,
                tunnel_ip: "10.44.0.5/32".to_string(),
                advertised_routes: Vec::new(),
                timestamp: 42,
            }),
            std::slice::from_ref(&receiver_pubkey),
        )
        .await
        .expect("private publish should succeed");

    let mut events = Vec::new();
    for _ in 0..50 {
        events = relay.events_snapshot().await;
        let current_count = events
            .iter()
            .filter(|event| event.kind == u32::from(NOSTR_KIND_NOSTR_VPN))
            .count();
        if current_count >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let current_events = events
        .iter()
        .filter(|event| event.kind == u32::from(NOSTR_KIND_NOSTR_VPN))
        .collect::<Vec<_>>();

    assert_eq!(
        current_events.len(),
        2,
        "expected current hello and private events"
    );
    assert!(
        events
            .iter()
            .all(|event| event.kind == u32::from(NOSTR_KIND_NOSTR_VPN))
    );

    sender.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn receiver_ignores_legacy_signal_events() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-legacy-rollout".to_string();
    let sender_keys = Keys::generate();
    let receiver_keys = Keys::generate();
    let sender_pubkey = sender_keys.public_key().to_hex();
    let receiver_pubkey = receiver_keys.public_key().to_hex();

    let sender_client = ClientBuilder::new()
        .signer(sender_keys.clone())
        .database(nostr_sdk::database::MemoryDatabase::new())
        .build();
    sender_client
        .add_relay(&relay_url)
        .await
        .expect("sender add relay");
    sender_client.connect().await;

    let receiver = NostrSignalingClient::new_with_keys(
        network_id.clone(),
        receiver_keys,
        vec![sender_pubkey.clone(), receiver_pubkey.clone()],
    )
    .expect("receiver client");
    receiver
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("receiver connect");

    tokio::time::sleep(Duration::from_millis(200)).await;

    publish_legacy_hello(&sender_client, &sender_keys)
        .await
        .expect("legacy hello publish should succeed");

    assert!(
        timeout(Duration::from_millis(400), receiver.recv())
            .await
            .is_err()
    );

    let announcement = PeerAnnouncement {
        node_id: "legacy-node".to_string(),
        public_key: "legacy-public".to_string(),
        endpoint: "127.0.0.1:51820".to_string(),
        local_endpoint: None,
        public_endpoint: None,
        tunnel_ip: "10.44.0.7/32".to_string(),
        advertised_routes: Vec::new(),
        timestamp: 77,
    };
    publish_legacy_private(
        &sender_client,
        &sender_keys,
        &network_id,
        &receiver_pubkey,
        SignalPayload::Announce(announcement.clone()),
    )
    .await
    .expect("legacy private publish should succeed");

    assert!(
        timeout(Duration::from_millis(400), receiver.recv())
            .await
            .is_err()
    );

    let _ = sender_client.disconnect().await;
    receiver.disconnect().await;
    relay.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn targeted_private_publish_only_reaches_requested_recipient() {
    let mut relay = WsRelay::new();
    relay.start().await.expect("relay should start");
    let relay_url = relay.url().expect("relay url");

    let network_id = "nostr-vpn-targeted-private".to_string();
    let sender_keys = Keys::generate();
    let receiver_a_keys = Keys::generate();
    let receiver_b_keys = Keys::generate();
    let sender_pubkey = sender_keys.public_key().to_hex();
    let receiver_a_pubkey = receiver_a_keys.public_key().to_hex();
    let receiver_b_pubkey = receiver_b_keys.public_key().to_hex();

    let participants = vec![
        sender_pubkey.clone(),
        receiver_a_pubkey.clone(),
        receiver_b_pubkey.clone(),
    ];
    let sender =
        NostrSignalingClient::new_with_keys(network_id.clone(), sender_keys, participants.clone())
            .expect("sender client");
    let receiver_a = NostrSignalingClient::new_with_keys(
        network_id.clone(),
        receiver_a_keys,
        participants.clone(),
    )
    .expect("receiver a client");
    let receiver_b = NostrSignalingClient::new_with_keys(network_id, receiver_b_keys, participants)
        .expect("receiver b client");

    sender
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("sender connect");
    receiver_a
        .connect(std::slice::from_ref(&relay_url))
        .await
        .expect("receiver a connect");
    receiver_b
        .connect(&[relay_url])
        .await
        .expect("receiver b connect");

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
        .publish_to(
            SignalPayload::Announce(announcement.clone()),
            std::slice::from_ref(&receiver_a_pubkey),
        )
        .await
        .expect("targeted private publish should succeed");

    let received = timeout(Duration::from_secs(5), receiver_a.recv())
        .await
        .expect("timed out waiting for targeted message")
        .expect("message expected");
    assert_eq!(received.payload, SignalPayload::Announce(announcement));

    let missing = timeout(Duration::from_millis(500), receiver_b.recv()).await;
    assert!(
        missing.is_err(),
        "non-targeted participant should not receive private announce"
    );

    sender.disconnect().await;
    receiver_a.disconnect().await;
    receiver_b.disconnect().await;
    relay.stop().await;
}

async fn publish_legacy_hello(client: &nostr_sdk::Client, keys: &Keys) -> anyhow::Result<()> {
    let expiration = Timestamp::now() + Duration::from_secs(300);
    let event = EventBuilder::new(
        Kind::Custom(LEGACY_SIGNAL_KIND),
        "",
        vec![
            Tag::identifier("hello"),
            Tag::custom(
                nostr_sdk::prelude::TagKind::SingleLetter(
                    nostr_sdk::prelude::SingleLetterTag::lowercase(nostr_sdk::prelude::Alphabet::L),
                ),
                vec!["hello".to_string()],
            ),
            Tag::expiration(expiration),
        ],
    )
    .to_event(keys)?;

    let output = client.send_event(event).await?;
    anyhow::ensure!(
        !output.success.is_empty(),
        "legacy hello rejected by all relays"
    );
    Ok(())
}

async fn publish_legacy_private(
    client: &nostr_sdk::Client,
    sender_keys: &Keys,
    network_id: &str,
    recipient_pubkey_hex: &str,
    payload: SignalPayload,
) -> anyhow::Result<()> {
    let recipient_pubkey = PublicKey::from_hex(recipient_pubkey_hex)?;
    let envelope = SignalEnvelope {
        network_id: network_id.to_string(),
        sender_pubkey: sender_keys.public_key().to_hex(),
        payload,
    };
    let plaintext = serde_json::to_string(&envelope)?;
    let encrypted = nip44::encrypt(
        sender_keys.secret_key(),
        &recipient_pubkey,
        &plaintext,
        nip44::Version::V2,
    )?;
    let expiration = Timestamp::now() + Duration::from_secs(300);
    let event = EventBuilder::new(
        Kind::Custom(LEGACY_SIGNAL_KIND),
        encrypted,
        vec![
            Tag::identifier(format!("private:{network_id}:{recipient_pubkey_hex}")),
            Tag::public_key(recipient_pubkey),
            Tag::expiration(expiration),
        ],
    )
    .to_event(sender_keys)?;

    let output = client.send_event(event).await?;
    anyhow::ensure!(
        !output.success.is_empty(),
        "legacy private rejected by all relays"
    );
    Ok(())
}
