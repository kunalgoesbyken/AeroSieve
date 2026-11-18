export class ClipPlayer {
  #context;
  #buffer;
  #source;

  load(samples, sampleRate) {
    this.stop();
    this.#context ??= new AudioContext();
    this.#buffer = this.#context.createBuffer(1, samples.length, sampleRate);
    this.#buffer.copyToChannel(samples, 0);
  }

  play(startS, durationS, onEnd) {
    if (!this.#buffer) return;
    this.stop();
    this.#context.resume();
    const source = this.#context.createBufferSource();
    source.buffer = this.#buffer;
    source.connect(this.#context.destination);
    source.onended = () => {
      if (this.#source === source) this.#source = undefined;
      onEnd?.();
    };
    source.start(0, startS, durationS);
    this.#source = source;
  }

  stop() {
    const source = this.#source;
    this.#source = undefined;
    source?.stop();
  }
}
