"""Stream a WAV file to aerosieve-serve at real-time pace and print each verdict.

    pip install websockets
    python stream_wav.py recording.wav --url ws://127.0.0.1:8080/v1/stream

The file must be mono 16-bit PCM at the rate the server announces.
"""

import argparse
import asyncio
import json
import wave

import websockets

FRAME_MS = 20


async def stream(path: str, url: str, realtime: bool) -> None:
    with wave.open(path, "rb") as audio:
        rate, channels, width = audio.getframerate(), audio.getnchannels(), audio.getsampwidth()
        pcm = audio.readframes(audio.getnframes())

    async with websockets.connect(url) as socket:
        ready = json.loads(await socket.recv())
        if (channels, width, rate) != (1, 2, ready["sample_rate"]):
            raise SystemExit(
                f"need mono 16-bit audio at {ready['sample_rate']} Hz; "
                f"{path} is {channels} channel(s), {8 * width}-bit, {rate} Hz"
            )

        async def send() -> None:
            frame_bytes = rate * FRAME_MS // 1000 * width
            for offset in range(0, len(pcm), frame_bytes):
                await socket.send(pcm[offset : offset + frame_bytes])
                if realtime:
                    await asyncio.sleep(FRAME_MS / 1000)
            await socket.send(json.dumps({"type": "end"}))

        sender = asyncio.create_task(send())
        async for message in socket:
            event = json.loads(message)
            if event["type"] == "clip":
                print(
                    f"{event['start_s']:7.2f}s  {event['duration_s']:5.2f}s  "
                    f"{event['verdict']:<10}  snr {event['wada_snr_db']:5.1f} dB  "
                    f"decided at {event['decided_at_s']:.2f}s"
                )
            elif event["type"] == "summary":
                print(f"kept {event['accepted']} of {event['clips']} clips from {event['heard_s']:.1f}s")
                break
            else:
                raise SystemExit(event.get("message", event))
        await sender


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("wav")
    parser.add_argument("--url", default="ws://127.0.0.1:8080/v1/stream")
    parser.add_argument("--fast", action="store_true", help="send as fast as possible")
    args = parser.parse_args()
    asyncio.run(stream(args.wav, args.url, realtime=not args.fast))


if __name__ == "__main__":
    main()
