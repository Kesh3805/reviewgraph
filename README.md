# ReviewGraph

A graph-native, incremental, evidence-driven pull-request reviewer.

ReviewGraph keeps a persistent, incrementally updated CodeGraph of each repository. It models every pull request as a set of changed symbols, measures their impact through the graph, and selects a small, budgeted context. It then runs specialized AI reviewers. Every candidate finding is verified against repository evidence, including a base-vs-head comparison, before anything is published.

```
understand repository → model change → measure impact → select context → reason → verify evidence → comment
```

| Area | Location |
| --- | --- |
| Product requirements | [`docs/product/PRD.md`](docs/product/PRD.md) |
| Architecture | [`docs/architecture/target-architecture.md`](docs/architecture/target-architecture.md) |
| Decisions | [`docs/decisions/`](docs/decisions/) |
| Plan | [`docs/planning/MASTER_IMPLEMENTATION_PLAN.md`](docs/planning/MASTER_IMPLEMENTATION_PLAN.md) |
| Rust data plane | [`engine/`](engine/) |
| Control plane (NestJS) | [`apps/api/`](apps/api/) |
| Web UI (Next.js) | [`apps/web/`](apps/web/) |
