# The ESP-IDF version is NOT set in this file
# It is set by ESP_IDF_VERSION in systypes/Common/features.cmake (and can be overridden for a SysType in
# systypes/<SysType>/features.cmake) and is supplied when the image is built by "raft build"
# So this Dockerfile can't be built directly with "docker build"
ARG ESP_IDF_VERSION
FROM espressif/idf:${ESP_IDF_VERSION}
WORKDIR /project
# Install dependencies required for Node.js install
RUN apt-get update && apt-get install -y curl software-properties-common && \
    curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get update && apt-get install -y nodejs g++
# Verify the installation of the specific Node.js version
RUN node -v && npm -v
# Configure Git to recognize /project as a safe directory
RUN git config --global --add safe.directory /project
