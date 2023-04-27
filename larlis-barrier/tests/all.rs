use std::{iter::repeat_with, marker::PhantomData, net::SocketAddr, sync::Arc};

use bincode::Options;
use larlis_barrier::{Message, Service};
use larlis_core::{
    actor::{Drive, State},
    app::{Closure, PureState},
};
use serde::de::DeserializeOwned;
use tokio::{net::UdpSocket, spawn};

#[derive(Default)]
struct De<M>(PhantomData<M>);

impl<'i, M> PureState<'i> for De<M>
where
    M: DeserializeOwned,
{
    type Input = (SocketAddr, &'i [u8]);
    type Output<'output> = (SocketAddr, M) where Self: 'output;

    fn update(&mut self, input: Self::Input) -> Self::Output<'_> {
        let (addr, buf) = input;
        let message = bincode::options()
            .allow_trailing_bytes()
            .deserialize(buf)
            .unwrap();
        (addr, message)
    }
}

async fn use_barrier(socket: Arc<UdpSocket>, service: SocketAddr) -> larlis_barrier::Message<u16> {
    let local_message = socket.local_addr().unwrap().port();

    // 1. listen to barrier message
    let mut message = Drive::default();
    let state =
        De::default().install(Closure::from(|x: (_, Message<u16>)| x.1).install(message.state()));
    let mut ingress = larlis_udp::In {
        socket: socket.clone(),
        state,
    };
    let ingress = spawn(async move { ingress.start().await });

    // 2. tx
    larlis_udp::Out(socket).update((
        service,
        bincode::options().serialize(&local_message).unwrap(),
    ));

    // 3. wait for rx
    let mut message = message.recv().await.unwrap();
    ingress.abort();
    message.sort();
    message
}

async fn provide_barrier(socket: Arc<UdpSocket>, count: usize) {
    let egress = Closure::from(|(addr, message)| {
        let message = bincode::options().serialize(&message).unwrap();
        (addr, message)
    })
    .install(larlis_udp::Out(socket.clone()));
    let mut finished = Drive::default();
    let state = De::default().install(Service::<u16, _, _>::new(egress, finished.state(), count));
    let mut ingress = larlis_udp::In { socket, state };
    let ingress = spawn(async move { ingress.start().await });
    finished.recv().await.unwrap();
    ingress.abort();
}

#[tokio::test]
async fn sync_two() {
    const N: usize = 2;

    let service_socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await.unwrap());
    let service_addr = service_socket.local_addr().unwrap();
    println!("service {service_addr}");
    let service = spawn(provide_barrier(service_socket, N));

    let tasks = Vec::from_iter(
        repeat_with(|| {
            spawn(async move {
                let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await.unwrap());
                use_barrier(socket, service_addr).await
            })
        })
        .take(N),
    );
    let mut messages = Vec::new();
    for task in tasks {
        messages.push(task.await.unwrap());
    }
    println!("{messages:?}");
    assert_eq!(messages[0], messages[1]);

    service.await.unwrap()
}
