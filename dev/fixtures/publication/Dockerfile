FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241
RUN apt-get update \
    && apt-get install --no-install-recommends --yes ca-certificates git python3 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 fixture \
    && useradd --uid 10001 --gid 10001 --create-home fixture \
    && mkdir -p /var/lib/publication-fixture \
    && chown 10001:10001 /var/lib/publication-fixture
COPY dev/fixtures/publication/server.py /opt/fixture/server.py
COPY dev/fixtures/task-quality/source.pack dev/fixtures/task-quality/source.json /opt/fixture/
USER 10001:10001
EXPOSE 8080 9418
ENTRYPOINT ["python3", "/opt/fixture/server.py"]
