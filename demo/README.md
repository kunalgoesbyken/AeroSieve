---
title: AeroSieve
emoji: 🎙️
colorFrom: green
colorTo: gray
sdk: docker
app_port: 7860
pinned: false
---

# AeroSieve demo

Upload a recording and see which clips [AeroSieve](https://github.com/kunalgoesbyken/AeroSieve) would keep for training a speech model, and why the rest are rejected.

## Run locally

```bash
pip install maturin
pip install ./bindings/python
pip install -r demo/requirements.txt
python demo/app.py
```

## Deploy as a Hugging Face Space

Create a Space with the Docker SDK and push this folder's `README.md` and `Dockerfile` to it. The image builds AeroSieve from the GitHub repository.
