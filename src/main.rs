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

use futures::StreamExt as _;
use libp2p::{
    PeerId, kad, noise,
    swarm::{StreamProtocol, SwarmEvent},
    tcp, yamux,
};

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

    let peer_id = PeerId::random();
    println!("Searching for the closest peers to {peer_id}");
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
                if ok.peers.is_empty() {
                    anyhow::bail!("Query finished with no closest peers.")
                }

                println!("Query finished with closest peers: {:#?}", ok.peers);

                return Ok(());
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
