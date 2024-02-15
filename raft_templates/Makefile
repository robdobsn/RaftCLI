# Build Project
DOCKER ?= 1

# Detect whether the OS is Windows Subsystem for Linux (WSL)
ifeq ($(OS),Windows_NT)
	WSL ?= 0
else
	WSL ?= $(shell uname -r | grep -i microsoft > /dev/null && echo 1 || echo 0)
endif

# Set the ESP IDF version
ESP_IDF_PATH ?= ~/esp/esp-idf-v{{esp_idf_version}}

# Set the build base folders
BUILD_BASE_FOLDER ?= build
BUILD_CONFIG_BASE_DIR = systypes
BUILD_RAFT_ARTEFACTS_DIR ?= build_raft_artefacts

# Set the SysType
SYSTYPE ?= $(notdir $(firstword $(filter-out $(BUILD_CONFIG_BASE_DIR)/Common, $(wildcard $(BUILD_CONFIG_BASE_DIR)/*))))

# Set the build configuration folders based on the SysType
BUILD_CONFIG_DIR = $(BUILD_CONFIG_BASE_DIR)/$(SYSTYPE)
COMMON_CONFIG_DIR = $(BUILD_CONFIG_BASE_DIR)/Common

# Set the build directory
BUILD_DIR = $(BUILD_BASE_FOLDER)/$(SYSTYPE)
ROOTDIR = $(realpath $(CURDIR))

# Set the sdkconfig files
SDKCONFIG_DEFAULTS_FILE ?= $(BUILD_CONFIG_DIR)/sdkconfig.defaults
SDKCONFIG_FILE ?= $(BUILD_RAFT_ARTEFACTS_DIR)/sdkconfig

# Set the target binary
TARGET_BINARY = $(BUILD_DIR)/$(SYSTYPE).bin

# Set the build commands
DOCKER_EXEC = docker build -t raftbuilder . && docker run --rm -v $(ROOTDIR):/project -w /project raftbuilder
LOCAL_EXEC = . $(ESP_IDF_PATH)/export.sh
CMD ?= idf.py -B $(BUILD_DIR) build

# Prepare build commands depending on the OS
ifeq ($(WSL),1)
	SERIAL_MONITOR ?= raft.exe monitor
	PYTHON_FOR_FLASH ?= python.exe
	PORT ?= COM3
else
	SERIAL_MONITOR ?= raft monitor
	PYTHON_FOR_FLASH ?= python3
	PORT ?= /dev/ttyUSB0
endif

# Custom command to delete build folders
ifeq ($(DOCKER),1)
DELETE_BUILD_FOLDERS=\
	@echo "-------------------- Deleting build folders --------------------"; \
	$(DOCKER_EXEC) rm -rf ./build/ ./build_raft_artefacts/ || true
BUILD_TARGET=\
	@echo "-------------------- Building in Docker --------------------"; \
	$(DOCKER_EXEC) $(CMD)
else
DELETE_BUILD_FOLDERS=\
	@echo "-------------------- Deleting build folders --------------------"; \
	rm -rf ./build/ ./build_raft_artefacts/ || true
BUILD_TARGET=\
	@echo "-------------------- Building Locally --------------------"; \
	$(LOCAL_EXEC) && $(CMD)
endif

# Default target
all: build

# Dependencies of target binary
$(TARGET_BINARY): $(wildcard $(BUILD_CONFIG_DIR)/*) $(wildcard $(COMMON_CONFIG_DIR)/*) $(wildcard $(COMMON_CONFIG_DIR)/FSImage/*) $(wildcard $(COMMON_CONFIG_DIR)/WebUI*)
	@$(DELETE_BUILD_FOLDERS)

# Clean the build
clean:
	@$(DELETE_BUILD_FOLDERS)

# Build the project
build: $(TARGET_BINARY)
	@$(BUILD_TARGET)

# Flash the project
ifneq ($(SERIAL_MONITOR),)
flash: build
	@$(PYTHON_FOR_FLASH) $(BUILD_DIR)/_deps/raftcore-src/scripts/flashUsingPartitionCSV.py $(BUILD_RAFT_ARTEFACTS_DIR)/partitions.csv $(BUILD_DIR) $(SYSTYPE).bin $(PORT) -s $(SDKCONFIG_FILE) -f fs.bin
	$(SERIAL_MONITOR) $(PORT) -l
else
flash: build
	@$(PYTHON_FOR_FLASH) $(BUILD_DIR)/_deps/raftcore-src/scripts/flashUsingPartitionCSV.py $(BUILD_RAFT_ARTEFACTS_DIR)/partitions.csv $(BUILD_DIR) $(SYSTYPE).bin $(PORT) -s $(SDKCONFIG_FILE) -f fs.bin
endif

# Phony targets to avoid name clashes with files
.PHONY: build clean flash test
