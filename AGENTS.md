# ArcFlow Development Guide (AGENTS.md)

## Project

ArcFlow is an open-source cross-platform client for compatible electrostimulation devices.

The project targets:

- Desktop (Windows/macOS/Linux)
- Mobile (Android/iOS)
- Web (future)

The project is plugin-first and shares as much code as possible across platforms.

---

# Architecture

Always follow this architecture.

UI

- React
- TypeScript
- TailwindCSS
- shadcn/ui

Desktop

- Tauri 2

Mobile

- Tauri 2

Core

- Rust

Never duplicate business logic between platforms.

Business logic belongs in Rust whenever possible.

---

# Core Responsibilities

Rust is responsible for:

- Bluetooth
- Device protocol
- Encryption
- Wave engine
- Script engine
- Plugin runtime for WASM/JS plugins
- Database
- File management
- Synchronization

React is responsible for:

- UI
- State
- Navigation
- Forms
- Visualization

Never implement Bluetooth logic in React.

---

# Bluetooth

Bluetooth is accessed only through Rust.

React

↓

IPC

↓

Rust Core

↓

Platform BLE

↓

Device

Do not directly call Web Bluetooth or native BLE APIs from React unless specifically implementing the future Web version.

---

# Plugin System

Plugins are implemented as WASM or JavaScript modules.

Plugins must never access Bluetooth directly.

Plugins communicate only through Plugin API.

WASM/JS Plugin

↓

Plugin API

↓

Rust Core

↓

BLE

Every plugin must run inside a sandbox.

Design Plugin API to remain backward compatible.

---

# Project Structure

apps/

desktop/
mobile/
web/

crates/

core/
protocol/
plugin-runtime/
storage/
wave/
crypto/

packages/

ui/
shared/

docs/

---

# UI Rules

Use:

React

TypeScript

TailwindCSS

shadcn/ui

Prefer reusable components.

Never duplicate UI logic.

---

# State Management

Use:

TanStack Query

for server/device state.

Use:

Zustand

for UI state.

Avoid Redux.

---

# Database

Use SQLite.

Database access belongs to Rust.

Never access SQLite directly from React.

---

# Communication

UI communicates with Rust only through IPC.

Avoid exposing internal implementation details.

Design IPC to remain stable.

---

# Coding Principles

Prefer composition over inheritance.

Prefer explicit types.

Avoid unnecessary abstractions.

Avoid premature optimization.

Keep modules small.

Keep APIs stable.

Write self-documenting code.

---

# Error Handling

Never ignore errors.

Provide meaningful error messages.

Use Result in Rust.

Use typed errors in TypeScript.

---

# Logging

Use structured logging.

Every Bluetooth operation should be traceable.

---

# Testing

Every new feature should include:

Unit tests

Integration tests (when applicable)

For UI:

Use Playwright.

---

# Debugging

When debugging frontend:

Always inspect Console.

Always inspect Network.

Fix all Console errors before finishing.

Verify the final UI manually using browser automation.

---

# Performance

Avoid unnecessary React rerenders.

Avoid blocking the UI thread.

Bluetooth operations should be asynchronous.

---

# Documentation

Every public API requires documentation.

Complex modules require architecture comments.

---

# Goal

Always optimize for:

Maintainability

Cross-platform reuse

Plugin ecosystem

Long-term evolution

Never optimize only for short-term implementation speed.

## Agent skills

### Issue tracker

Issues and PRDs are tracked in GitHub Issues; external PRs are not a triage surface. See `docs/agents/issue-tracker.md`.

### Triage labels

Use the default five-label triage vocabulary. See `docs/agents/triage-labels.md`.

### Domain docs

Use a single-context domain-doc layout. See `docs/agents/domain.md`.
