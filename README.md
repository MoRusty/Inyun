# Inyun Engine

A game engine made using Rust programming language and modern Vulkan 1.3 graphics

---

## Overview

Inyun Engine is a 3D renderer built in Rust using Vulkan, designed to leverage modern graphics APIs alongside safe
systems programming. It serves as both a practical rendering engine and an exploration of Vulkan's architecture.

## Features

- **HDR rendering pipeline** — 16-bit intermediate render target for post-processing (bloom, tone mapping)
- **Dynamic rendering** — uses Vulkan 1.3's dynamic rendering extension, no legacy render pass boilerplate
- **gpu-allocator** — efficient GPU memory tracking and management
- **Validation layers** — built-in debug utilities for catching issues early
- **Modular architecture** — rendering context, swapchain, image layout transitions, and pipeline state are clearly
  separated

## Stack

- Rust
- Vulkan 1.3 (via [ash](https://github.com/ash-rs/ash))

## Building

```sh
cargo build
```

Requires a Vulkan 1.3 capable GPU and the Vulkan SDK installed.

## The Game

Inyun is an ECS-based 3D RTS where AI agents are a core gameplay mechanic, not just NPCs. The goal is a living, reactive
world where autonomous agents learn, collaborate, compete, and drive emergent behaviour — making every playthrough feel
different. The engine is built with this in mind from the start.

## Planned

- Compute shaders
- Post-processing stack
- Asset pipeline integration

