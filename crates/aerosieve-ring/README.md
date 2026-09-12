# aerosieve-ring

Lock-free SPSC ring buffer for transferring `AudioChunk` frames between producer and consumer without copies.

Backed by [`ringbuf`](https://crates.io/crates/ringbuf) (HeapRb). Each slot carries audio samples, transcript text, source metadata, and bitflag state.

---

**Status: not used by the v2 pipeline.** v2 processes whole decoded files
rather than streaming fixed-size frames, so there is no producer/consumer
handoff for this to sit in. Kept, tested, and building because it is a correct
SPSC ring that a future streaming front-end would want; it is not dead code
pretending to be load-bearing.
