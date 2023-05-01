use std::{iter::repeat_with, net::SocketAddr, sync::Arc};

use larlis_barrier::{Message, Service};
use larlis_bincode::{de, ser};
use larlis_core::{
    actor::{Drive, State},
    app::{Closure, PureState},
};

use tokio::{net::UdpSocket, spawn};

async fn use_barrier(socket: Arc<UdpSocket>, service: SocketAddr) -> larlis_barrier::Message<u16> {
    let local_message = socket.local_addr().unwrap().port();

    // 1. listen to barrier message
    let mut message = Drive::default();
    let state = de().install(Closure::from(|x: (_, Message<u16>)| x.1).install(message.state()));
    let mut ingress = larlis_udp::In {
        socket: socket.clone(),
        state,
    };
    let ingress = spawn(async move { ingress.start().await });

    // 2. tx
    ser()
        .install(larlis_udp::Out(socket))
        .update((service, local_message));

    // 3. wait for rx
    let mut message = message.recv().await.unwrap();
    ingress.abort();
    message.sort();
    message
}

async fn provide_barrier(socket: Arc<UdpSocket>, count: usize) {
    let egress = ser().install(larlis_udp::Out(socket.clone()));
    let mut finished = Drive::default();
    let state = de().install(Service::<u16, _, _>::new(egress, finished.state(), count));
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
