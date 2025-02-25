// Copyright 2018 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use std::time::Duration;

use futures::{
    StreamExt as _,
    future::{join, join_all},
};
use libp2p::{
    PeerId,
    kad::{self, PeerInfo},
    noise,
    swarm::{StreamProtocol, SwarmEvent, dial_opts::DialOpts, dummy},
    tcp, yamux,
};
use tokio::time::{Instant, sleep_until, timeout};

const BOOTNODES: [&str; 4] = [
    "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    "QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
    "QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
    "QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
];

const IPFS_PROTO_NAME: StreamProtocol = StreamProtocol::new("/ipfs/kad/1.0.0");

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let target_id = PeerId::random();
    tracing::info!(%target_id);
    let closest_peers = get_closest_peers(target_id).await?;
    if closest_peers.len() != 20 {
        anyhow::bail!("insufficient closest peers")
    }
    let _ = timeout(
        Duration::from_secs(5 * 60),
        probe_peer_group(target_id, closest_peers),
    )
    .await;
    Ok(())
}

async fn probe_peer_group(target_id: PeerId, closest_peers: Vec<PeerInfo>) {
    let start = Instant::now();
    let query_task = async {
        let mut probe_at = start + Duration::from_secs(60);
        loop {
            sleep_until(probe_at).await;
            probe_at += Duration::from_secs(60);
            let current_closest_peers = match get_closest_peers(target_id).await {
                Ok(peers) => peers,
                Err(err) => {
                    tracing::warn!(%err, "query");
                    continue;
                }
            };
            let count = closest_peers
                .iter()
                .filter(|peer| {
                    current_closest_peers
                        .iter()
                        .any(|other_peer| other_peer.peer_id == peer.peer_id)
                })
                .count();
            tracing::info!(at = ?(Instant::now() - start), %count, "query");
        }
    };
    let check_task = async {
        let mut probe_at = start + Duration::from_secs(60);
        loop {
            sleep_until(probe_at).await;
            probe_at += Duration::from_secs(60);
            let count = join_all(
                closest_peers
                    .iter()
                    .map(|peer| check_liveness(peer.clone())),
            )
            .await
            .into_iter()
            .filter(Result::is_ok)
            .count();
            tracing::info!(at = ?(Instant::now() - start), %count, "check")
        }
    };
    join(query_task, check_task).await;
}

async fn get_closest_peers(peer_id: PeerId) -> anyhow::Result<Vec<PeerInfo>> {
    // Create a random key for ourselves.
    // let local_key = libp2p::identity::Keypair::generate_ed25519();
    // let mut swarm = libp2p::SwarmBuilder::with_existing_identity(local_key.clone())
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|key| {
            // Create a Kademlia behaviour.
            let mut cfg = kad::Config::new(IPFS_PROTO_NAME);
            cfg.set_query_timeout(Duration::from_secs(5 * 60));
            let store = kad::store::MemoryStore::new(key.public().to_peer_id());
            kad::Behaviour::with_config(key.public().to_peer_id(), store, cfg)
        })?
        .build();

    // Add the bootnodes to the local routing table. `libp2p-dns` built
    // into the `transport` resolves the `dnsaddr` when Kademlia tries
    // to dial these nodes.
    for peer in BOOTNODES {
        swarm
            .behaviour_mut()
            .add_address(&peer.parse()?, "/dnsaddr/bootstrap.libp2p.io".parse()?);
    }

    swarm.behaviour_mut().get_closest_peers(peer_id);

    loop {
        let event = swarm.select_next_some().await;

        match event {
            SwarmEvent::Behaviour(kad::Event::OutboundQueryProgressed {
                result: kad::QueryResult::GetClosestPeers(Ok(ok)),
                ..
            }) => {
                // The example is considered failed as there
                // should always be at least 1 reachable peer.
                anyhow::ensure!(!ok.peers.is_empty(), "Query finished with no closest peers");
                tracing::trace!(%peer_id, ?ok.peers, "Query finished with closest peers");
                return Ok(ok.peers);
            }

            SwarmEvent::Behaviour(kad::Event::OutboundQueryProgressed {
                result:
                    kad::QueryResult::GetClosestPeers(Err(kad::GetClosestPeersError::Timeout {
                        ..
                    })),
                ..
            }) => {
                anyhow::bail!("Query for closest peers timed out")
            }

            event => tracing::trace!("{event:?}"),
        }
    }
}

async fn check_liveness(peer: PeerInfo) -> anyhow::Result<()> {
    let mut swarm = libp2p::SwarmBuilder::with_new_identity()
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_quic()
        .with_dns()?
        .with_behaviour(|_| dummy::Behaviour)?
        .build();
    swarm.dial(
        DialOpts::peer_id(peer.peer_id)
            .addresses(peer.addrs)
            .build(),
    )?;
    loop {
        let event = swarm.select_next_some().await;

        match event {
            SwarmEvent::Dialing {
                peer_id: Some(peer_id),
                ..
            } => tracing::debug!(%peer_id, "dialing"),

            SwarmEvent::ConnectionEstablished {
                peer_id: connected_id,
                ..
            } => {
                anyhow::ensure!(connected_id == peer.peer_id);
                tracing::info!(%peer.peer_id, "peer alive");
                return Ok(());
            }

            SwarmEvent::OutgoingConnectionError { error, .. } => {
                tracing::warn!(%error);
                anyhow::bail!(error)
            }

            event => tracing::trace!("{event:?}"),
        }
    }
}
