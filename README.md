# Iceon

**A native Kubernetes IDE, built entirely in Rust.**

Iceon is a modern, native desktop environment for working with Kubernetes. Designed to make cluster management, workload inspection, resource orchestration, and day-to-day Kubernetes workflows feel intuitive without sacrificing the power and flexibility developers expect.

No Electron. No web frontend. No runtime built around JavaScript.

Just Rust, native performance, and a workspace designed for Kubernetes.

<p align="center">
  <em>Understand your clusters. Manage your workloads. Stay in control.</em>
</p>

---

## Why Iceon?

Kubernetes is powerful, but working with it doesn't always have to mean switching between terminals, YAML files, dashboards, and scattered tools.

Iceon aims to bring the Kubernetes development experience into one cohesive desktop environment.

Instead of being another graphical wrapper around `kubectl`, Iceon is designed as a dedicated Kubernetes IDE — a place where developers can explore clusters, understand workloads, inspect resources, and manage their infrastructure through a unified interface.

## Design Philosophy

- **Native by design** — Built entirely with Rust, without Tauri, Electron, or a JavaScript-based frontend.
- **Developer-first** — Designed around real Kubernetes workflows rather than simplified abstractions.
- **Clarity over complexity** — Make complex cluster states easier to understand and navigate.
- **Performance matters** — A lightweight desktop experience that feels responsive even when working with large clusters.
- **Open and extensible** — Built with an architecture that can grow alongside the Kubernetes ecosystem.

## Vision & Roadmap

Iceon is being developed as a comprehensive Kubernetes workspace.

### Cluster Management
- Connect to multiple Kubernetes clusters.
- Manage kubeconfig contexts.
- Switch between clusters and namespaces.
- Monitor cluster health and resource utilization.
- Inspect nodes and cluster-level resources.

### Workload Explorer
- Browse Pods, Deployments, StatefulSets, DaemonSets, and Jobs.
- Inspect workload details and lifecycle events.
- View container logs in real time.
- Access interactive terminal sessions.
- Monitor resource consumption.

### Kubernetes Resource Management
- Explore Kubernetes resources through a structured interface.
- Inspect and edit YAML manifests.
- Apply and manage resource configurations.
- View relationships between resources.
- Track resource status and events.

### Developer Experience
- Integrated terminal.
- Resource search and filtering.
- Real-time status updates.
- Context-aware resource inspection.
- Streamlined workflows for debugging and troubleshooting.

### Observability
- Cluster and workload monitoring.
- Event exploration.
- Log inspection.
- Resource utilization insights.
- A unified view of operational health.

*These are project goals, not a representation that every feature is already implemented.*

## Built with Rust

Iceon started with a Tauri + React architecture. As the project evolved, the direction changed toward a fully native Rust implementation.

The goal is to keep the entire application stack within Rust, from its core application logic to its desktop interface.

This means:
- No React frontend.
- No Tauri runtime.
- No Electron.
- No Node.js application runtime.

A native architecture gives Iceon greater control over performance, resource usage, application lifecycle, and integration with the underlying operating system.

## Project Status

<em>Early development — actively evolving.</em>

Iceon is still under development. APIs, architecture, interfaces, and features may change as the project matures.

The current priority is establishing a solid native foundation before expanding the feature set.

## Roadmap

- [ ] Establish the native Rust desktop architecture.
- [ ] Build the core application interface.
- [ ] Implement Kubernetes connectivity.
- [ ] Introduce cluster and namespace navigation.
- [ ] Build workload exploration.
- [ ] Add YAML inspection and editing.
- [ ] Integrate logs and terminal workflows.
- [ ] Expand monitoring and observability capabilities.
- [ ] Improve multi-cluster workflows.

## Contributing

Iceon is intended to grow as an open-source project.

Contributions, discussions, feature proposals, and architectural feedback are welcome.

Whether you're a Rust developer, Kubernetes enthusiast, or someone who wants a better desktop Kubernetes experience, there's room to contribute.

## License

License information will be added as the project evolves.

---

<p align="center">
  <strong>Iceon</strong><br/>
  A better way to work with Kubernetes.
</p>
