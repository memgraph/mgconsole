
FROM debian:trixie-slim AS builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y \
    git \
    cmake \
    make \
    gcc \
    g++ \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

ARG GIT_REF=master

RUN git clone https://github.com/memgraph/mgconsole.git /mgconsole && \
    git -C /mgconsole checkout --detach "$GIT_REF"

WORKDIR /mgconsole

RUN cmake -B build -DCMAKE_BUILD_TYPE=Release . && \
    cmake --build build && \
    cmake --install build --strip

FROM gcr.io/distroless/base-debian13:debug

WORKDIR /mgconsole

COPY --from=builder /mgconsole/build/src/mgconsole /usr/local/bin/mgconsole

ENTRYPOINT ["mgconsole"]
