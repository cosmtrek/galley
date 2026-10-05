# syntax=docker/dockerfile:1

FROM node:24-bookworm-slim AS web
WORKDIR /src/web
RUN corepack enable
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
COPY examples/ /src/examples/
RUN pnpm build

# The server embeds web/dist at compile time, so it is built after the workbench.
FROM rust:1.88-slim-bookworm AS server
WORKDIR /src/server
COPY server/ ./
COPY web/src/styles/report.css /src/web/src/styles/report.css
COPY --from=web /src/web/dist /src/web/dist
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin galley \
    && mkdir /data && chown galley:galley /data
COPY --from=server /src/server/target/release/galley /usr/local/bin/galley
USER galley
ENV GALLEY_ADDR=0.0.0.0:7860 \
    GALLEY_DATA=/data
VOLUME /data
EXPOSE 7860
CMD ["galley"]
