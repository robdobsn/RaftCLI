# The ESP-IDF version is normally set by ESP_IDF_VERSION in systypes/Common/features.cmake (and can be
# overridden for a SysType in systypes/<SysType>/features.cmake) and is supplied as a build-arg when the
# image is built by "raft build", which overrides the default below.
# The default (set by "raft new" to match features.cmake) lets "docker build" work standalone and keeps
# Docker's InvalidDefaultArgInFrom lint quiet.
ARG ESP_IDF_VERSION=v{{esp_idf_version}}
FROM espressif/idf:${ESP_IDF_VERSION}
WORKDIR /project
# Install dependencies required for Node.js install
RUN apt-get update && apt-get install -y curl software-properties-common && \
    curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get update && apt-get install -y nodejs g++
# Verify the installation of the specific Node.js version
RUN node -v && npm -v
# Configure Git to treat all repos (the project and ESP-IDF with its submodules) as safe - system-wide so it
# applies whichever user the container runs as ("raft build" also sets this via environment variables)
RUN git config --system --add safe.directory '*'
