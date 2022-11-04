FROM rust:1.64.0-slim as builder

RUN apt-get update

RUN apt-get install libssl-dev pkg-config -y

WORKDIR /home/cpc-nat-traversal

COPY . .

RUN cargo update

RUN cargo install --path .

# Runtime container
FROM debian:buster-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl libssl-dev && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/cpc-nat-traversal /usr/bin/cpc-nat-traversal
ENTRYPOINT [ "cpc-nat-traversal" ]
