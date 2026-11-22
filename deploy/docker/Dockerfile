FROM debian:13-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       ca-certificates \
       openssh-client \
       tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 rtkk \
    && useradd --uid 10001 --gid 10001 --create-home --shell /usr/sbin/nologin rtkk \
    && install -d -m 0755 -o 10001 -g 10001 /data

COPY target/release/rtkk /usr/local/bin/rtkk
RUN chmod 0755 /usr/local/bin/rtkk

USER 10001:10001
WORKDIR /data
EXPOSE 19912

HEALTHCHECK --interval=10s --timeout=3s --start-period=10s --retries=12 \
  CMD ["/usr/local/bin/rtkk", "healthcheck", "--url", "http://127.0.0.1:19912/api/v1/health"]

ENTRYPOINT ["/usr/local/bin/rtkk"]
CMD ["serve", "--listen", "0.0.0.0:19912", "--work-dir", "/data"]
