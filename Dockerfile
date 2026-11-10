# Build:  docker build -t lighthouse .
# Setup:  docker run -it --rm -v ./data:/data lighthouse setup
# Run:    docker run -d --name lighthouse --restart unless-stopped -v ./data:/data lighthouse
#         (put lighthouse.toml in ./data; set `database = "/data/lighthouse.db"` in it)

FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p lighthouse

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 lighthouse
COPY --from=build /src/target/release/lighthouse /usr/local/bin/lighthouse
# Containers are updated by pulling a new image; the built-in updater only reports new versions.
ENV LIGHTHOUSE_NO_SELF_UPDATE=1
USER lighthouse
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8651
ENTRYPOINT ["lighthouse", "--config", "/data/lighthouse.toml"]
CMD ["run"]
