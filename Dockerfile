# 砾（LI）可复现构建环境（H2）
#
# 只用于主机与固件构建，不进目标镜像（D012）。
#
# 构建与验证：
#   docker build -t jishi-build .
#   docker run --rm jishi-build
#
# 设计取舍：
#   - host gcc 用发行版包（build-essential），arm-none-eabi 用 xPack 压缩包，
#     与 tools/toolchains/toolchain.lock.json 的目录布局一致
#     （<root>/arm-none-eabi/bin/arm-none-eabi-gcc），build-firmware.ps1 的
#     $tcRoot 推导才能成立。
#   - 容器镜像内的 arm-none-eabi 版本由 lock 的 URL 固定；host gcc 是发行版版本，
#     与开发机锁定的 MinGW 16.1.0 不一致，因此设置 JM_SKIP_TOOLCHAIN_CHECK=1
#     （体积/行为口径以开发机实测为准，容器只用于「能否构建」的复现）。
#   - 本镜像未在本机实测（开发机无 Docker）；命令与边界见 docs/工具链.md。

FROM mcr.microsoft.com/powershell:7.4-ubuntu-22.04

ENV DEBIAN_FRONTEND=noninteractive \
    JM_SKIP_TOOLCHAIN_CHECK=1 \
    JM_TOOLCHAIN_ROOT=/opt/jishi-toolchains \
    JM_HOST_GCC=/usr/bin/gcc \
    JM_ARM_GCC=/opt/jishi-toolchains/arm-none-eabi/bin/arm-none-eabi-gcc \
    JM_ARM_SIZE=/opt/jishi-toolchains/arm-none-eabi/bin/arm-none-eabi-size

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl unzip tar \
        rustc cargo \
    && rm -rf /var/lib/apt/lists/*

# arm-none-eabi：用 lock 里固定的 xPack 包（Linux x64，约 280 MB），布局与绿色包一致
RUN mkdir -p /opt/jishi-toolchains /tmp/tc \
    && curl -fsSL -o /tmp/arm.tgz \
       "https://github.com/xpack-dev-tools/arm-none-eabi-gcc-xpack/releases/download/v13.3.1-1.1/xpack-arm-none-eabi-gcc-13.3.1-1.1-linux-x64.tar.gz" \
    && tar -xzf /tmp/arm.tgz -C /tmp/tc \
    && mv /tmp/tc/* /opt/jishi-toolchains/arm-none-eabi \
    && rm -rf /tmp/arm.tgz /tmp/tc

WORKDIR /src
COPY . .

# 主机构建（含 micc 与 jmrun）；固件交叉编译用 `类` 示例（CI 同款大包）
RUN pwsh -NoProfile -File tools/build.ps1 -SkipTests \
    && pwsh -NoProfile -File tools/build-firmware.ps1 -Example 类

CMD ["pwsh", "-NoProfile", "-Command", "Write-Host '构建产物：compiler/target/release/micc.exe、build/jmrun.exe、build/stm32/jishim.bin'"]
