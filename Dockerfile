
FROM debian:bullseye-slim AS builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y \
    git \
    cmake \
    make \
    gcc \
    g++ \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN git clone https://github.com/memgraph/mgconsole.git /mgconsole

WORKDIR /mgconsole

RUN mkdir build && cd build && \
    cmake -DCMAKE_BUILD_TYPE=Release .. && \
    make && \
    make install

FROM debian:bullseye-slim

WORKDIR /mgconsole

COPY --from=builder /mgconsole/build/src/mgconsole /usr/local/bin/mgconsole

ENTRYPOINT ["mgconsole"]
