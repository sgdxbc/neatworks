use std::{iter::repeat_with, net::SocketAddr};

use larlis_barrier::{Message, Service};
use larlis_bincode::{de, ser};
use larlis_core::{
    actor::{Drive, State},
    app::{Closure, PureState},
};

use tokio::spawn;

type UserPayload = u16;

async fn use_barrier(addr: SocketAddr, service: SocketAddr) -> Message<UserPayload> {
    // 1. listen to barrier message
    let mut message = Drive::<Message<UserPayload>>::default();
    let state = de().install(Closure::from(|(_, message)| message).install(message.state()));
    let out = larlis_udp::Out::bind(addr).await;
    let local_message = out.0.local_addr().unwrap().port();
    let mut ingress = larlis_udp::In::new(&out, state);
    let ingress = spawn(async move { ingress.start().await });

    // 2. tx
    ser().install(out).update((service, local_message));

    // 3. wait for rx
    let mut message = message.recv().await.unwrap();
    ingress.abort();
    message.sort();
    message
}

async fn provide_barrier(addr: SocketAddr, count: usize) {
    let out = larlis_udp::Out::bind(addr).await;
    let egress = ser().install(out.clone());
    let mut finished = Drive::default();
    let state = de().install(Service::<UserPayload, _, _>::new(
        egress,
        finished.state(),
        count,
    ));
    let mut ingress = larlis_udp::In::new(&out, state);
    let ingress = spawn(async move { ingress.start().await });
    finished.recv().await.unwrap();
    ingress.abort();
}

#[tokio::test]
async fn sync_two() {
    const N: usize = 2;

    let service_addr = SocketAddr::from(([0, 0, 0, 0], 60000));
    let service = spawn(provide_barrier(service_addr, N));

    let tasks = Vec::from_iter(
        repeat_with(|| {
            spawn(use_barrier(
                SocketAddr::from(([0, 0, 0, 0], 0)),
                service_addr,
            ))
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
