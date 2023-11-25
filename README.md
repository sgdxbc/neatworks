## Modeling Concurrent Stateful Applications with Sessions

> Move to dedicated document if desired.

Stateful applications are usually implemented as event handlers that are driven by a tight event loop. When demanding concurrency, such model only permits detached concurrent tasks because there's no way to customize event loop to receive notification of concurrent tasks finish, which is undesirable.

Concurrent applications are usually implemented as stateless, *microservice* style tasks, which incurs error-prone and hard-to-tweak locking issues when dealing with any shared mutable states.

This codebase tries to develop a programming pattern that is suitable for concurrent stateful applications. Code snippets are grouped into *sessions*, which contain logic that connected with causal dependencies. Code in different sessions are causally independent to each other, thus share no state and can be concurrently executed. Each session is an asynchronous task (or coroutine) that drives its own event loop. A session has full control of what to be expected from event loop and even when to block on receiving events.

The goal is to have *streamlined* code. The code skeleton is like:

```rust
async fn a_session(
    // immutable arguments
    // message channels and transportations
) -> crate::Result<()> {
    // declare mutable states

    // everything here happens sequentially, the code comes later executes later
}

async fn b_session() {
    // everything here is concurrent to everything in `a_session`
}

// example of complicated sessions which may take too many arguments as a 
// function
struct CState {
    // immutable arguments
    // message channels and transportations
    // mutable states
}

impl CState {
    pub fn new(
        // immutable arguments
        // message channels and transportations
    ) -> Self {
        Self {
            // move arguments
            // initialize mutable states
        }
    }

    pub async fn session(&mut self) {
        // similar to above
    }
}
```

**Common patterns of sessions.** A session run may contain arbitrary behaviors. However, the following patterns commonly appear as far as have seen, and the codebase provides several helpers are provided for them.

Collector sessions. This is the most common and recommended pattern. These sessions collect information by receiving messages from (potentially multiple) channels and joining results from other sessions, until "reduce" the information into returning result. The sessions may be multistaged, collecting information from difference sources in different stages, and unnecessary to be in "one big event loop" style. These sessions determine their termination base on their local information, so do not require a shutdown signal input.

Examples of the tasks of these sessions:

* (Verify and) collect quorum certificates
* Collect client requests and a produce block/ordering batch
* Transaction workload that issues one or more requests through a client sequentially. The workload may also verify intermediate results and conditionally issue requests base on results of previous requests
* Close-loop benchmark that repeatedly joins the above workload for certain duration
Generally these are the sessions with streaming input and one-shot output.

Listener sessions. This is the opposite of the previous pattern, and these are sessions with one-shot input and streaming output. The example of these sessions is network socket listeners. These sessions usually loop forever, so they need to take an explicit shutdown signal on spawning.

Server sessions. This is a variant of collector session, with only a slight difference that these sessions usually run forever by default and need to take a shutdown signal. (There are also cases where a shutdown message is sent through network and the server shut itself down.) Everything else is the same. These sessions collect network messages and return trivial results (or some metrics information).

Service sessions. These are sessions that take streaming input and produce *corresponded* streaming output. Every output is replying one of the input. If the output and input are not correlated, it is possible that the session is incorrectly abstracted, and the design needs to be revised. The shutdown condition of these sessions are simply the closing of input source.

**The usage of Tokio.** The codebase relies on Tokio for most core features. Most essentially, concurrent primitives `spawn` and `select!` are deeply baked into sessions logic, kind of like extensions of the Rust language. Other than this, asynchronous channels are massively used with trivial wrapping for error handling. The background task management mimics `JoinSet`, but implemented with channels to enable sharing. The network functions and time utilities are also used, but only as normal dependent library.
