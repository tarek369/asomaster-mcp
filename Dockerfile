# Builds and runs the asomaster-mcp bridge so MCP directories can check that it starts and lists
# its tools. The bridge needs the ASOMaster desktop app (macOS or Windows) to do real work.
FROM rust:1.99-alpine3.22 AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked --bin asomaster-mcp

FROM alpine:3.22
COPY --from=build /src/target/release/asomaster-mcp /usr/local/bin/asomaster-mcp
ENV ASOMASTER_NO_LAUNCH=1
ENTRYPOINT ["/usr/local/bin/asomaster-mcp"]
