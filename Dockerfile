# Runs the asomaster-mcp bridge so MCP directories can check that it starts and lists its tools.
# The bridge needs the ASOMaster desktop app (macOS or Windows) to do real work.
FROM alpine:3.20
ARG VERSION=0.1.7
ARG SHA256=70a8ebea36d862e7a8bc7e246412e91ab4fc443c949982c550db9f20cc3778eb
RUN wget -q -O /usr/local/bin/asomaster-mcp \
      "https://github.com/tarek369/asomaster-mcp/releases/download/v${VERSION}/asomaster-mcp-x86_64-unknown-linux-musl" \
 && echo "${SHA256}  /usr/local/bin/asomaster-mcp" | sha256sum -c - \
 && chmod +x /usr/local/bin/asomaster-mcp
ENV ASOMASTER_NO_LAUNCH=1
ENTRYPOINT ["/usr/local/bin/asomaster-mcp"]
