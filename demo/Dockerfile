FROM python:3.12-slim

RUN apt-get update     && apt-get install -y --no-install-recommends build-essential curl git     && rm -rf /var/lib/apt/lists/*
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /app
RUN git clone --depth 1 https://github.com/kunalgoesbyken/AeroSieve.git source     && pip install --no-cache-dir ./source/bindings/python     && pip install --no-cache-dir -r source/demo/requirements.txt     && cp source/demo/app.py app.py

ENV GRADIO_SERVER_NAME=0.0.0.0 GRADIO_SERVER_PORT=7860
EXPOSE 7860
CMD ["python", "app.py"]
