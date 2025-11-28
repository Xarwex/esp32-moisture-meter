FROM mcr.microsoft.com/devcontainers/rust:2.0.3-trixie

RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash && \
    cargo binstall cargo-espflash espflash espup

RUN espup install

