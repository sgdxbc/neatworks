## Concurrent Stateful Applications

> Move to dedicated document if desired.

Stateful applications are usually implemented as event handlers that are driven by a tight event loop. When demanding concurrency, such model only permits detached concurrent tasks because there's no way to customize event loop to receive notification of concurrent tasks finish, which is undesirable.

Concurrent applications are usually implemented as stateless, *microservice* style tasks, which incurs error-prone and hard-to-tweak locking issues when dealing with any shared mutable states.

This codebase tries to develop a programming pattern that is suitable for concurrent stateful applications. Code snippets are grouped into *sessions*, which contain logic that connected with causal dependencies. Code in different sessions are causally independent to each other, thus share no state and can be concurrently executed. Each session is an asynchronous task (or coroutine) that drives its own event loop. A session has full control of what to be expected from event loop and even when to block on receiving events.

**The usage of Tokio.** The codebase relies on Tokio in two different aspects. Firstly, concurrency is encoded through `spawn` and `select!`, which as used as concurrency primitives. Applications are directly coupled with them.

Secondly, Tokio is also used as a library for message passing with channels provided by `tokio::sync` and interacting with external world through transportation implementation and time primitives. These are mostly decoupled with applications and could be interchangeable with other libraries if desired.

**Shared reusable modules.** The described programming pattern requires low or no framework support by definition. Nevertheless, several modules are provided to decouple applications from external dependencies and to keep them universal.

* `src/model.rs` general abstractions and encapsulations for implementing applications, currently including transportation abstraction and various channel types.
* `src/task.rs` useful helpers for working with background tasks.
* `src/crypto.rs` wire and in-memory format for digitally signed messages, and signing/verifying operations.