# Graph-Native AI Pull Request Reviewer

## Product Requirements Document

**Working product name:** ReviewGraph  
**Document type:** Product Requirements Document / System Product Specification  
**Status:** Proposed  
**Primary use case:** Automated pull-request review  
**Product category:** Developer Infrastructure / Repository Intelligence / AI Code Review  
**Target integrations:** GitHub, GitLab, Bitbucket  
**Initial reference implementation:** Existing prototype CodeGraph and repository-intelligence infrastructure

---

# 1. Executive Summary

ReviewGraph is a graph-native AI pull-request reviewer designed around persistent repository intelligence.

The product should provide a review experience comparable to a strong senior engineer who already understands:

- the repository architecture,
- dependency relationships,
- execution paths,
- coding conventions,
- testing patterns,
- security boundaries,
- previous design decisions,
- historical regressions,
- and the intent of the current pull request.

The fundamental architecture is:

```text
Repository
    │
    ▼
Persistent Repository Intelligence
    │
    ├── AST
    ├── Symbols
    ├── Dependency Graph
    ├── API Graph
    ├── Test Relationships
    ├── Configuration
    ├── Semantic Index
    └── Repository Profile
    │
    ▼
Pull Request
    │
    ▼
Incremental Change Model
    │
    ▼
Impact Graph
    │
    ▼
Targeted Context Selection
    │
    ▼
Specialized Reviewers
    │
    ▼
Evidence Verification
    │
    ▼
High-Confidence Findings
```

The system must explicitly avoid the naive architecture:

```text
git diff
   ↓
large LLM prompt
   ↓
comments
```

Instead:

```text
Repository
    ↓
Repository Intelligence
    ↓
CodeGraph
    ↓
PR Change Model
    ↓
Impact Analysis
    ↓
Targeted Context
    ↓
Specialized Reviewers
    ↓
Verification
    ↓
Deduplication
    ↓
High-confidence PR comments
```

The defining product thesis is:

> **PR review quality improves substantially when the reviewer understands code relationships instead of merely reading changed lines.**

The second product thesis is:

> **Repository understanding should be computed incrementally and reused across pull requests rather than rediscovered for every review.**

The third is:

> **A smaller number of evidence-backed findings is more valuable than a large volume of speculative comments.**

The intended output is therefore not:

```text
20 observations
15 false positives
5 useful comments
```

but preferably:

```text
5 candidates
3 survive verification
2 are genuinely actionable
2 comments are posted
```

ReviewGraph should optimize for developer trust rather than comment volume.

---

# 2. Product Vision

ReviewGraph should become a reusable **repository intelligence platform**.

Pull-request review is the first application built on top of it.

Long term, the same intelligence layer should support:

```text
Repository Intelligence Platform
│
├── Pull Request Review
├── Change Impact Analysis
├── Architecture Enforcement
├── Test Selection
├── Refactoring Assistance
├── Codebase Exploration
├── Security Analysis
├── Dependency Risk Analysis
├── Migration Planning
├── Documentation Intelligence
└── Engineering Analytics
```

The architectural relationship should therefore be:

```text
                   Repository Intelligence
                           Platform
                              │
            ┌─────────────────┼─────────────────┐
            │                 │                 │
            ▼                 ▼                 ▼
        PR Review        Code Search       Impact Analysis
            │
            ▼
       ReviewGraph
```

Review logic must not become inseparably coupled to repository parsing or CodeGraph construction.

---

# 3. Product Positioning

The target positioning is:

> **CodeRabbit-style PR review, but graph-native, incremental, repository-aware, evidence-driven, and designed to minimize false positives.**

Key differentiators:

1. Persistent CodeGraph.
2. Incremental graph updates.
3. Symbol-level diff understanding.
4. Dependency-aware impact analysis.
5. Repository-specific conventions.
6. Reviewer specialization.
7. Evidence-backed verification.
8. Explicit review budgets.
9. Historical review intelligence.
10. Deterministic analysis before LLM reasoning.
11. Strong cache reuse.
12. Architecture designed for arbitrary repositories.

---

# 4. Problem Statement

Current AI pull-request reviewers commonly suffer from five architectural limitations.

## 4.1 Diff blindness

Most tools primarily reason over:

```text
changed lines
+
nearby code
```

They often lack knowledge of:

```text
callers
callees
interfaces
implementations
tests
configuration
API consumers
database interactions
cross-module dependencies
runtime boundaries
```

A locally valid modification can therefore introduce a system-level regression invisible from the diff itself.

---

## 4.2 Excessive repository retrieval

A naive reviewer may provide the model with large quantities of repository code.

This causes:

- higher latency,
- higher model cost,
- weaker attention,
- irrelevant context,
- duplicated reasoning,
- context-window pressure,
- and more hallucinated relationships.

The problem is not lack of context.

It is poor context selection.

---

## 4.3 Repository rediscovery

Many review systems repeatedly rediscover repository structure for each pull request.

For large repositories this is wasteful.

Repository architecture changes much more slowly than pull requests.

Therefore:

```text
repository understanding
```

should be persistent, while:

```text
PR understanding
```

should be incremental.

---

## 4.4 False-positive saturation

An AI reviewer loses developer trust quickly when it produces comments such as:

```text
This might potentially cause...
Consider possibly...
You may want to...
```

without demonstrating an actual defect.

ReviewGraph should treat every LLM finding as an **untrusted candidate finding** until independently verified.

---

## 4.5 Generic review behavior

Repositories have local engineering rules not always written down.

Examples:

- every mutating service method uses transactions,
- controllers never access repositories directly,
- every queue job uses deterministic IDs,
- external API calls require retry wrappers,
- authorization is performed through one service,
- all domain errors use a particular hierarchy.

A useful reviewer must infer and understand these patterns.

---

# 5. Product Goals

ReviewGraph must:

1. Understand an arbitrary supported repository automatically.
2. Build persistent repository intelligence once.
3. Incrementally update repository intelligence as code changes.
4. Convert line-level diffs into semantic symbol-level changes.
5. determine which unaffected code may be impacted by a PR.
6. Retrieve only context relevant to the changed behavior.
7. Run specialized review strategies in parallel.
8. Use deterministic tooling wherever possible.
9. Verify candidate findings against repository evidence.
10. Suppress weak, speculative, or redundant comments.
11. Produce concise, developer-grade inline findings.
12. Learn repository conventions without blindly enforcing accidental patterns.
13. Integrate with major source-control platforms.
14. operate efficiently on large repositories.
15. remain independent of any single consumer codebase.

---

# 6. Non-Goals

The first versions should not attempt to become:

- a complete IDE,
- a replacement compiler,
- a full SAST suite,
- a dependency vulnerability database,
- an autonomous merge system,
- an autonomous code-writing agent,
- a generic project-management platform,
- a full CI/CD system,
- an unrestricted repository agent.

ReviewGraph may consume output from these systems but should not duplicate mature tooling unnecessarily.

---

# 7. Primary Users

## 7.1 Individual developer

Wants fast, useful PR feedback before human review.

Primary needs:

- correctness detection,
- test gaps,
- architectural violations,
- understandable explanations.

---

## 7.2 Pull-request reviewer

Wants assistance finding issues that are difficult to discover through manual diff inspection.

Primary needs:

- impact analysis,
- call-path context,
- dependency changes,
- cross-file effects.

---

## 7.3 Tech lead / principal engineer

Wants architectural consistency.

Primary needs:

- boundary violations,
- repository convention enforcement,
- risky coupling,
- missing abstractions,
- regression-prone modules.

---

## 7.4 Security engineer

Wants changed attack surfaces highlighted.

Primary needs:

- authorization changes,
- trust-boundary changes,
- secret handling,
- dangerous input paths,
- data exposure,
- injection surfaces.

---

## 7.5 Platform engineering team

Wants scalable, deterministic review infrastructure.

Primary needs:

- predictable latency,
- low model cost,
- observability,
- reproducibility,
- policy controls.

---

# 8. Core User Experience

A repository should require minimal setup.

Example:

```bash
review init
```

The system should:

```text
Clone / inspect repository
        │
        ▼
Detect repository environment
        │
        ▼
Parse supported languages
        │
        ▼
Build symbol model
        │
        ▼
Build CodeGraph
        │
        ▼
Discover tests/config/APIs
        │
        ▼
Build repository profile
        │
        ▼
Persist repository intelligence
```

After initialization:

```bash
review pr 184
```

or via source-control webhook:

```text
Pull Request Opened / Updated
            │
            ▼
ReviewGraph
```

ReviewGraph performs incremental analysis and publishes findings.

---

# 9. End-to-End Product Flow

```text
                    GitHub / GitLab / Bitbucket
                              │
                              ▼
                         Pull Request
                              │
                    ┌─────────┴─────────┐
                    │                   │
                  Diff              Repository
                    │                   │
                    └─────────┬─────────┘
                              ▼
                    Repository Initializer
                              │
                  ┌───────────┴───────────┐
                  │                       │
             Detect Stack            Build Index
                  │                       │
                  ▼                       ▼
             Language AST            CodeGraph
                  │                       │
                  └───────────┬───────────┘
                              ▼
                    Graph + Diff Engine
                              │
                              ▼
                  Changed Symbol Detection
                              │
                              ▼
                    Impact / Context Graph
                              │
                              ▼
                       Review Engine
                              │
             ┌────────────────┼────────────────┐
             ▼                ▼                ▼
        Correctness       Security          Quality
             │                │                │
             └────────────────┼────────────────┘
                              ▼
                     Finding Verification
                              │
                              ▼
                     Dedup + Prioritization
                              │
                              ▼
                    Inline PR Comments
```

---

# 10. System Architecture

The recommended top-level package architecture is:

```text
packages/
├── core/
│   ├── repository/
│   ├── pull-request/
│   ├── diff/
│   ├── change-model/
│   ├── review/
│   ├── findings/
│   └── policy/
│
├── codegraph/
│   ├── builder/
│   ├── parser/
│   ├── nodes/
│   ├── edges/
│   ├── query/
│   ├── incremental/
│   ├── snapshots/
│   └── storage/
│
├── languages/
│   ├── typescript/
│   ├── javascript/
│   ├── python/
│   ├── java/
│   ├── go/
│   └── rust/
│
├── context/
│   ├── selector/
│   ├── ranking/
│   ├── expansion/
│   ├── compression/
│   └── budgets/
│
├── analysis/
│   ├── static/
│   ├── graph/
│   ├── heuristics/
│   ├── semantic/
│   └── risk/
│
├── reviewers/
│   ├── correctness/
│   ├── security/
│   ├── tests/
│   ├── performance/
│   ├── architecture/
│   └── maintainability/
│
├── verification/
│   ├── evidence/
│   ├── contradiction/
│   ├── reproduction/
│   ├── confidence/
│   └── deduplication/
│
├── repository-profile/
│   ├── conventions/
│   ├── architecture/
│   ├── tests/
│   ├── security/
│   └── history/
│
├── integrations/
│   ├── github/
│   ├── gitlab/
│   └── bitbucket/
│
├── llm/
│   ├── gateway/
│   ├── prompts/
│   ├── models/
│   ├── routing/
│   └── cache/
│
├── persistence/
│   ├── metadata/
│   ├── graph/
│   ├── cache/
│   └── artifacts/
│
├── workers/
│   ├── indexing/
│   ├── review/
│   ├── verification/
│   └── publishing/
│
└── cli/
```

Applications may then be:

```text
apps/
├── api/
├── worker/
├── cli/
└── dashboard/
```

---

# 11. Architectural Principle: Ports and Adapters

The system must isolate generic repository intelligence from provider-specific integrations.

Core interfaces should resemble:

```ts
interface RepositoryProvider {
  getRepository(...): Promise<Repository>;
  getPullRequest(...): Promise<PullRequest>;
  getDiff(...): Promise<Diff>;
  getFile(...): Promise<FileContents>;
}

interface ReviewPublisher {
  publishInlineFinding(...): Promise<void>;
  publishSummary(...): Promise<void>;
}

interface LanguageAnalyzer {
  supports(file: SourceFile): boolean;
  parse(file: SourceFile): Promise<ParsedUnit>;
}

interface GraphStore {
  query(...): Promise<GraphResult>;
  upsertNodes(...): Promise<void>;
  upsertEdges(...): Promise<void>;
}

interface ModelGateway {
  reason(...): Promise<ModelResult>;
}
```

GitHub, GitLab, Bitbucket, local repositories, and consumer repositories should plug into these abstractions.

---

# 12. Repository Initialization

## 12.1 Command

```bash
review init
```

Optional:

```bash
review init --repository .
review init --provider github
review init --force
```

---

# 13. Initialization Responsibilities

Initialization must detect:

1. languages,
2. frameworks,
3. package managers,
4. build systems,
5. test frameworks,
6. source roots,
7. test roots,
8. generated-code directories,
9. dependency manifests,
10. monorepo workspaces,
11. public entry points,
12. API routes,
13. command-line entry points,
14. worker entry points,
15. database schemas/migrations,
16. infrastructure configuration,
17. authentication and authorization boundaries where detectable,
18. linting configuration,
19. compiler configuration,
20. CI configuration,
21. architecture metadata,
22. documentation likely relevant to engineering rules.

---

# 14. Persistent Repository State

Default local representation:

```text
.review/
├── config.yaml
├── repository.json
│
├── graph/
│   ├── nodes/
│   ├── edges/
│   ├── indexes/
│   └── metadata/
│
├── ast/
│
├── symbols/
│
├── semantic/
│
├── profile/
│   ├── architecture.json
│   ├── conventions.json
│   ├── testing.json
│   ├── security.json
│   └── dependencies.json
│
├── snapshots/
│
├── history/
│
└── cache/
```

Production hosted deployments may store these objects remotely.

The filesystem layout is an implementation detail.

The logical model is the stable contract.

---

# 15. Repository Fingerprinting

Every indexed repository state should have a fingerprint.

Example inputs:

```text
repository ID
commit SHA
language analyzer version
graph schema version
configuration hash
parser version
repository profile version
```

This enables:

- cache validation,
- reproducibility,
- selective invalidation,
- migrations,
- stale-index detection.

---

# 16. CodeGraph

The CodeGraph is the primary structural intelligence model.

It must represent more than files.

---

# 17. Core Node Types

Initial node taxonomy:

```text
Repository
Package
Module
Directory
File

Namespace
Class
Interface
Struct
Trait
Enum
TypeAlias

Function
Method
Constructor
Property
Field
Parameter
Variable
Constant

APIEndpoint
Controller
Handler
Middleware

DatabaseEntity
DatabaseTable
DatabaseColumn
Migration

Queue
QueueProducer
QueueConsumer
JobHandler

Configuration
EnvironmentVariable

TestSuite
TestCase
Fixture

ExternalDependency
ExternalAPI

BuildTarget
CLICommand
Worker

DocumentationRule
ArchitecturalBoundary
```

Language-specific extensions should be allowed.

---

# 18. Core Edge Types

```text
CONTAINS
DECLARES

IMPORTS
EXPORTS

CALLS
CALLED_BY

READS
WRITES

IMPLEMENTS
EXTENDS

OVERRIDES
REFERENCES

USES_TYPE
RETURNS_TYPE
ACCEPTS_TYPE

ROUTES_TO
HANDLED_BY

TESTS
COVERS

PRODUCES_JOB
CONSUMES_JOB

READS_CONFIG
WRITES_CONFIG

READS_TABLE
WRITES_TABLE

DEPENDS_ON
DEPENDED_ON_BY

THROWS
CATCHES

SERIALIZES
DESERIALIZES

VALIDATES
AUTHORIZES

PUBLISHES
SUBSCRIBES
```

The graph schema must be versioned.

---

# 19. Graph Metadata

Nodes and edges should support metadata such as:

```text
source location
symbol ID
language
confidence
visibility
ownership
module
commit introduced
last modified commit
complexity
public/private
generated/not-generated
framework metadata
test status
```

Certain inferred relationships should carry a confidence score.

For example:

```text
STATIC_CALL_EDGE       confidence = 1.0
DYNAMIC_INFERRED_CALL  confidence = 0.62
```

This prevents speculative graph edges from being treated as ground truth.

---

# 20. Stable Symbol Identity

Incremental graph updates require stable symbol identifiers.

A symbol ID must not rely solely on line numbers.

Possible identity components:

```text
repository
language
module
qualified name
symbol kind
signature
```

For example:

```text
typescript:
src/auth/auth.service.ts
AuthService.authorize(User, Resource): Promise<boolean>
```

Renames should preferably be recognized as symbol transformations rather than deletion plus unrelated addition.

---

# 21. Incremental CodeGraph

This is a fundamental product requirement.

The system must never rebuild the complete repository graph for every pull request unless forced by invalidation conditions.

Base model:

```text
Commit N
   │
   ▼
Graph(N)
```

PR changes produce:

```text
Graph(base)
   +
Changed Files
   │
   ▼
Reparse Changed Units
   │
   ▼
Changed Symbols
   │
   ▼
Recompute Affected Edges
   │
   ▼
Invalidate Dependent Derived Data
   │
   ▼
Graph(head)
```

---

# 22. Incremental Update Algorithm

For every changed file:

```text
File modified
    │
    ▼
Compare content fingerprint
    │
    ▼
Parse new AST
    │
    ▼
Extract symbols
    │
    ▼
Compare old/new symbols
    │
    ├── unchanged
    ├── modified
    ├── added
    ├── removed
    └── renamed/moved
    │
    ▼
Update nodes
    │
    ▼
Update outgoing edges
    │
    ▼
Find dependent inbound edges
    │
    ▼
Recompute affected relationships
    │
    ▼
Invalidate related analysis cache
```

---

# 23. Graph Invalidation

Invalidation must be dependency-aware.

Example:

```text
AuthService.authorize()
changed
```

Possible invalidations:

```text
AuthService.authorize AST cache
AuthService.authorize semantic summary
direct caller context cache
authorization execution-path cache
tests mapped to authorize()
review summaries containing authorize()
```

It must not automatically invalidate unrelated modules.

---

# 24. Full Rebuild Conditions

Full rebuild should occur only when required.

Examples:

- graph schema migration,
- parser version incompatible with stored representation,
- repository configuration substantially changed,
- source roots changed,
- graph corruption detected,
- user explicitly requests rebuild.

Command:

```bash
review graph rebuild
```

---

# 25. Diff Engine

The diff layer must produce both textual and semantic changes.

Inputs:

```text
base commit
head commit
provider diff metadata
```

Output:

```text
PullRequestChangeModel
```

---

# 26. Pull Request Change Model

The PR must be normalized into a first-class domain object.

Example:

```ts
interface PullRequestChangeModel {
  files: ChangedFile[];
  symbols: ChangedSymbol[];
  APIs: ChangedAPI[];
  dependencies: ChangedDependency[];
  schemas: ChangedSchema[];
  configs: ChangedConfiguration[];
  tests: ChangedTest[];
  riskSignals: RiskSignal[];
}
```

---

# 27. Symbol-Level Diff Mapping

Example source diff:

```diff
- return permissionService.check(user, resource);
+ return user.role === 'admin';
```

The engine should map this to:

```text
File:
src/auth/auth.service.ts

Class:
AuthService

Method:
authorize()

Semantic changes:
- removed call to PermissionService.check
- added direct role comparison
- authorization dependency removed
- behavior branch modified
```

This is far more useful than:

```text
line 81 changed
```

---

# 28. AST Change Classification

Changed AST nodes should be classified.

Examples:

```text
control_flow_changed
exception_handling_changed
authorization_changed
database_write_changed
return_type_changed
API_contract_changed
validation_removed
dependency_added
dependency_removed
call_added
call_removed
condition_changed
loop_changed
async_behavior_changed
transaction_boundary_changed
```

Language analyzers may provide specialized classifications.

---

# 29. Change Intent Classification

The system should estimate change intent using deterministic and semantic signals.

Possible classes:

```text
feature
bugfix
refactor
test-only
documentation
configuration
dependency
migration
performance
security
generated-code
```

The value should influence review strategy but never be treated as infallible.

---

# 30. Impact Analysis

Impact analysis asks:

> What existing behavior may be affected by this changed symbol?

For each changed symbol, gather:

```text
direct callers
direct callees
transitive callers within budget
implementations
interfaces
subclasses
overrides
related types
API entry points
database interactions
configuration dependencies
tests
queue consumers/producers
external API boundaries
```

---

# 31. Impact Graph

For a changed symbol:

```text
authorize()
   │
   ├── CALLER → updateUser()
   │       │
   │       └── CALLER → UserController.update()
   │
   ├── CALLEE → PermissionService.check()
   │
   ├── TESTED_BY → authorize.test.ts
   │
   └── IMPLEMENTS → AuthProvider.authorize()
```

This derived graph is the immediate reasoning surface for reviewers.

---

# 32. Graph-Based Context Selection

The LLM should never receive the repository indiscriminately.

Context must be deliberately selected.

For every review task:

```text
Changed Symbols
      │
      ▼
Graph Expansion
      │
      ▼
Candidate Context
      │
      ▼
Relevance Ranking
      │
      ▼
Review Budget
      │
      ▼
Compressed Context Package
```

---

# 33. Context Signals

Ranking should combine several independent signals.

Conceptually:

```text
relevance =
  structural_relationship
+ graph_proximity
+ execution_path_relevance
+ changed_code_similarity
+ test_relationship
+ API_relationship
+ configuration_relationship
+ semantic_similarity
+ historical_signal
+ risk_importance
```

No single signal should dominate globally.

---

# 34. Graph Distance

Useful default heuristic:

```text
distance 0 → changed symbol
distance 1 → usually include
distance 2 → conditionally include
distance 3 → include only with strong supporting signal
distance 4+ → generally exclude
```

Graph distance is a heuristic, not the context algorithm.

---

# 35. Context Budgets

Every review task should have an explicit budget.

Example:

```text
Correctness reviewer
- 8 changed symbols
- maximum 20 related symbols
- maximum 8 tests
- maximum 4 configuration files

Security reviewer
- only trust-boundary-related symbols
- authorization paths
- validation logic
- external inputs
```

The system must be able to stop graph expansion before context becomes unbounded.

---

# 36. Context Compression

Large files should be represented through structured excerpts whenever possible.

Instead of:

```text
entire 1,500-line service
```

provide:

```text
class signature
changed method
related methods
called interfaces
selected caller
relevant invariants
```

Compression must preserve source locations so evidence can be traced back.

---

# 37. Risk Classification

Before expensive review work, each PR and symbol should receive risk signals.

Examples:

```text
authentication
authorization
cryptography
permissions
payments
database migrations
schema changes
transaction boundaries
concurrency
queues
background jobs
public API contracts
serialization
validation
external network calls
filesystem access
secrets
dependency updates
```

---

# 38. Risk Score Usage

Risk should control:

```text
review depth
context budget
reviewer selection
verification depth
model selection
test inspection
confidence threshold
```

Risk should not automatically mean a defect exists.

---

# 39. Low-Risk Change Suppression

Examples of changes normally deserving reduced review:

```text
formatting
comments
pure renames
generated snapshots
simple constants
mechanical import sorting
```

The system should still detect when a nominally simple change affects public contracts or behavior.

---

# 40. Deterministic Analysis Pipeline

The system should maximize deterministic analysis before model reasoning.

Pipeline:

```text
Compiler / Parser
       ↓
Static Analysis
       ↓
Graph Analysis
       ↓
Repository Rules
       ↓
Heuristic Checks
       ↓
LLM Reasoning
       ↓
Evidence Verification
```

Examples of deterministic tools:

```text
compiler
linter
type checker
dependency analyzer
test discovery
security scanner
AST queries
graph queries
schema analysis
```

The LLM should reason about questions difficult to prove mechanically.

---

# 41. Multi-Stage Review Architecture

A single giant review prompt should not exist.

Architecture:

```text
                       PR
                        │
                        ▼
                 Change Analyzer
                        │
          ┌─────────────┼─────────────┐
          │             │             │
          ▼             ▼             ▼
    Correctness      Security     Architecture
          │             │             │
          ▼             ▼             ▼
      Reviewer       Reviewer      Reviewer
          │             │             │
          └─────────────┼─────────────┘
                        │
             ┌──────────┼──────────┐
             ▼          ▼          ▼
            Tests   Performance  Maintainability
             │          │          │
             └──────────┼──────────┘
                        ▼
                   Aggregation
                        │
                        ▼
                   Verification
                        │
                        ▼
                    Findings
```

---

# 42. Correctness Reviewer

Responsibilities:

- logic defects,
- broken invariants,
- invalid state transitions,
- missing branches,
- incorrect error handling,
- behavior changes affecting callers,
- unsafe null assumptions,
- incorrect async behavior,
- transaction errors,
- race conditions,
- resource lifecycle problems.

It should prioritize concrete behavior changes over stylistic observations.

---

# 43. Security Reviewer

Responsibilities:

- authorization bypasses,
- authentication regressions,
- input validation,
- injection paths,
- data exposure,
- unsafe deserialization,
- trust-boundary changes,
- credential handling,
- path traversal,
- SSRF surfaces,
- insecure configuration,
- privilege escalation.

It should use repository-specific security patterns where available.

---

# 44. Test Reviewer

Responsibilities:

- changed behavior with no relevant tests,
- assertions that no longer cover modified behavior,
- tests coupled to implementation details,
- missing negative tests,
- missing concurrency tests,
- broken fixtures,
- stale mocks,
- insufficient regression protection.

---

# 45. Performance Reviewer

Responsibilities:

- obvious N+1 behavior,
- accidentally unbounded loops,
- expensive repeated operations,
- synchronous I/O introduced into hot paths,
- unnecessary full-table operations,
- cache invalidation errors,
- queue or batch inefficiencies,
- query regressions.

It should not speculate about performance without a plausible execution path.

---

# 46. Architecture Reviewer

Responsibilities:

- architectural boundary violations,
- forbidden dependencies,
- cross-layer coupling,
- module ownership violations,
- bypassing established abstractions,
- circular dependency risks,
- duplicate architecture,
- state ownership confusion.

Repository profile data is especially important here.

---

# 47. Maintainability Reviewer

Responsibilities:

- unnecessary complexity,
- duplicated logic,
- unreadable control flow,
- dangerous abstraction leakage,
- unreviewable functions,
- inconsistent repository patterns.

This reviewer should have a high threshold for comments because style noise is particularly damaging.

---

# 48. Reviewer Routing

Not every reviewer should execute on every PR.

Example:

```text
README-only change
→ no security review
→ no performance review

Database migration
→ correctness
→ architecture
→ database safety
→ tests

AuthService change
→ correctness
→ security
→ tests
→ architecture
```

Routing reduces cost and noise.

---

# 49. Candidate Finding Model

A reviewer never creates a final comment directly.

It creates:

```ts
interface CandidateFinding {
  category: FindingCategory;
  title: string;
  description: string;

  changedLocation: SourceLocation;

  evidence: Evidence[];
  affectedSymbols: SymbolId[];

  severityCandidate: Severity;
  confidenceCandidate: number;

  reviewer: ReviewerType;
  reasoningArtifacts: FindingArtifact[];
}
```

Candidates are untrusted.

---

# 50. Finding Verification

Verification is mandatory for externally published findings.

Pipeline:

```text
Candidate finding
      │
      ▼
Changed-code anchor exists?
      │
      ▼
Graph relationship exists?
      │
      ▼
Repository evidence exists?
      │
      ▼
Behavior already existed before PR?
      │
      ▼
Contradictory evidence?
      │
      ▼
Actionable?
      │
      ▼
Confidence threshold
      │
      ▼
Publish / suppress
```

---

# 51. Base-vs-Head Verification

One of the most important verification steps is:

> Did the PR introduce the problem?

For every candidate:

```text
base behavior
     vs
head behavior
```

If the issue already exists identically in the base branch, the finding should normally not be published as a PR regression.

Possible exception:

- PR meaningfully expands exposure to the existing bug.

---

# 52. Evidence Model

Evidence may include:

```text
changed source
caller path
callee path
test behavior
interface contract
configuration
database schema
repository convention
compiler diagnostic
lint result
static-analysis result
historical regression
```

Every published finding must have at least one strong evidence source.

---

# 53. Contradiction Pass

Before publication, ask:

```text
What evidence would make this finding wrong?
```

Examples:

- existing authorization happens upstream,
- framework guarantees the invariant,
- wrapper already catches the error,
- transaction is owned by the caller,
- generated code should not be reviewed,
- type system makes the state impossible.

This adversarial step should substantially reduce false positives.

---

# 54. Confidence Model

Confidence should not be equal to model self-confidence.

It should combine evidence signals.

Conceptually:

```text
confidence =
  changed_code_anchor
+ deterministic_evidence
+ graph_evidence
+ repository_evidence
+ reproduction_strength
+ reviewer_agreement
- contradictory_evidence
- inference_uncertainty
```

---

# 55. Finding Thresholds

Possible policy:

```text
confidence < 0.55
→ suppress

0.55–0.70
→ internal finding only

0.70–0.85
→ publish if medium/high impact

> 0.85
→ publish normally
```

Exact thresholds should be calibrated empirically.

---

# 56. Deduplication

Multiple reviewers may discover the same underlying problem.

Example:

```text
Security:
authorization check removed

Correctness:
permission validation removed

Architecture:
PermissionService bypassed
```

These should become one finding.

Deduplication should use:

```text
source location
affected symbol
root cause
semantic similarity
evidence overlap
```

The output should contain one strongest explanation.

---

# 57. Finding Prioritization

Priority should consider:

```text
severity
confidence
blast radius
public exposure
security impact
data integrity impact
number of affected callers
test coverage
change risk
```

Do not produce artificial severity inflation merely because many callers exist.

---

# 58. Review Comment Format

Comments should resemble experienced engineering review feedback.

Example:

```text
🔴 High — Authorization check bypassed

`authorize()` now accepts `user.role === "admin"` directly and no longer
calls `PermissionService.check()`.

`AdminService.updateUser()` reaches this path from the user-management
endpoint, so resource-level permissions are no longer evaluated for that
operation.

Evidence:
AuthService.authorize()
  → AdminService.updateUser()
  → UserController.update()

Preserve the resource permission check before permitting the mutation.
```

Avoid:

```text
This potentially might...
Consider whether...
Maybe this could...
```

unless genuine uncertainty is material and clearly explained.

---

# 59. Comment Requirements

Every inline finding should answer:

1. What changed?
2. Why is it incorrect or risky?
3. What concrete path proves relevance?
4. What behavior can result?
5. What corrective direction is appropriate?

---

# 60. Comment Suppression

The system should avoid comments for:

- personal stylistic preferences,
- formatting handled by linters,
- obvious generated code,
- speculative hypothetical edge cases,
- unrelated pre-existing issues,
- trivial restatement of the diff,
- architectural opinions without repository evidence,
- duplicate findings.

---

# 61. Review Summary

Every reviewed PR should optionally receive a summary:

```text
ReviewGraph

Changed:
- 17 files
- 8 behavioral symbols
- 2 API contracts

Risk areas:
- authentication
- database writes

Findings:
- 1 high
- 1 medium

Verified:
2 / 6 candidate findings

Suppressed:
4 low-confidence candidates
```

Developer-facing output should not expose internal reasoning traces.

---

# 62. Repository Profile

Every initialized repository should have an automatically generated profile.

```text
Repository Profile
├── languages
├── frameworks
├── architecture
├── module boundaries
├── testing conventions
├── naming conventions
├── error handling
├── dependency patterns
├── API conventions
├── security conventions
├── persistence conventions
├── queue conventions
└── historical patterns
```

---

# 63. Repository Convention Discovery

Convention inference should inspect repeated patterns.

Example:

```text
38 service methods that write to DB
37 use transaction wrapper
1 new method does not
```

This may indicate an architectural rule.

But the system must distinguish:

```text
intentional convention
```

from:

```text
incidental repetition
```

---

# 64. Convention Confidence

Each inferred convention should carry:

```text
sample count
consistency ratio
scope
exceptions
confidence
last recalculated
```

Example:

```json
{
  "rule": "controllers_do_not_access_repositories",
  "scope": "src/modules/**",
  "samples": 86,
  "violations": 1,
  "confidence": 0.96
}
```

---

# 65. Explicit Rules Override Inference

Priority:

```text
explicit repository policy
        >
architecture documentation
        >
established code convention
        >
generic engineering guidance
```

Explicit repository instructions should dominate inferred conventions.

---

# 66. Repository-Specific Rules

Configuration might allow:

```yaml
rules:
  forbidden_dependencies:
    - from: controllers
      to: repositories

  queue_jobs:
    require_deterministic_id: true

  database:
    migrations_only: true

  tests:
    public_api_changes_require_tests: true
```

---

# 67. Historical Review Intelligence

If provider history is available:

```text
Past pull requests
      │
      ▼
Past findings
      │
      ├── accepted
      ├── rejected
      ├── resolved
      ├── dismissed
      └── ignored
      │
      ▼
Repository Review Signals
```

---

# 68. Historical Signals

Examples:

```text
this repository consistently rejects this pattern
this module frequently regresses
API changes normally require contract tests
reviewers frequently request transaction handling here
a previous similar finding was dismissed
```

These are signals, not rules.

---

# 69. Anti-Reinforcement Requirement

Historical learning must never blindly amplify previous reviewer behavior.

The system must prevent:

```text
one bad review
→ stored as policy
→ repeated forever
```

Historical signals should require:

- repetition,
- supporting repository evidence,
- explicit acceptance,
- or human-configured policy.

---

# 70. Review Feedback Loop

Where supported, developers should be able to mark findings as:

```text
useful
false positive
already handled
not relevant
intentional
```

This should contribute to calibration.

---

# 71. Cache Architecture

Aggressive caching is essential.

Required caches:

```text
AST cache
symbol cache
graph cache
dependency cache
semantic summary cache
embedding cache
context cache
review cache
verification cache
repository profile cache
```

---

# 72. Cache Keys

Keys should contain sufficient versioning.

Example:

```text
repo:{repoId}
commit:{sha}
symbol:{symbolId}
analyzer:{version}
graphSchema:{version}
config:{hash}
```

Avoid cache entries whose correctness depends on hidden state.

---

# 73. Review Cache

Review results may be reused when:

```text
same base
same head
same review configuration
same repository profile
same analyzer versions
```

A model-version change may invalidate only model-derived layers rather than structural analysis.

---

# 74. Parallel Execution

Independent reviewers should run concurrently.

```text
                   PR Change Model
                         │
          ┌──────────────┼──────────────┐
          ▼              ▼              ▼
      Security      Correctness       Tests
          │              │              │
          ├──────────────┼──────────────┤
          ▼              ▼              ▼
    Architecture    Performance   Maintainability
          │              │              │
          └──────────────┼──────────────┘
                         ▼
                    Aggregator
```

Verification may also run concurrently by finding.

---

# 75. Work Scheduling

Recommended high-level queues:

```text
repository-index
incremental-index
pr-analysis
review-task
finding-verification
review-publish
history-ingest
```

Queue payloads should contain IDs and pointers rather than large source blobs.

---

# 76. Idempotency

All review stages must be safely retryable.

Example identifiers:

```text
repo-index:{repositoryId}:{commitSha}

pr-review:
{provider}:{repo}:{pr}:{headSha}

reviewer:
{prReviewId}:{reviewerType}:{inputHash}

verification:
{candidateFindingId}:{verificationVersion}
```

---

# 77. Cancellation

When a new commit is pushed to an open PR:

```text
review head A running
        │
new head B arrives
        │
        ▼
cancel/supersede A
        │
        ▼
incrementally process B
```

No obsolete comments should be published after supersession.

---

# 78. Repository Provider Integration

Provider adapter responsibilities:

```text
authenticate
fetch repository metadata
fetch pull request
fetch diff
fetch changed files
fetch commit metadata
publish comments
update comments
resolve stale comments
handle webhook signatures
```

Core review logic must not depend on provider-specific payload structures.

---

# 79. GitHub Integration

Initial integration capabilities:

```text
GitHub App installation
PR opened
PR reopened
PR synchronized
review requested
manual review command
inline review comments
summary check
commit status
```

---

# 80. GitLab Integration

Equivalent abstraction:

```text
Merge Request
webhook
discussion comments
pipeline/report integration
```

---

# 81. Bitbucket Integration

Equivalent abstraction:

```text
Pull Request
webhook
inline comment
build status
```

---

# 82. Local CLI Mode

A local mode is essential for development and private repositories.

Examples:

```bash
review init

review diff HEAD~1

review branch feature/auth dev

review pr 184

review graph inspect AuthService.authorize

review impact src/auth/auth.service.ts:authorize

review profile

review doctor
```

---

# 83. `review doctor`

Should validate:

```text
repository initialized
graph current
parser availability
provider authentication
model configuration
cache health
storage health
language support
config validity
```

---

# 84. Graph Debugging

Graph-native products require strong introspection.

Commands:

```bash
review graph symbol AuthService.authorize

review graph callers AuthService.authorize

review graph callees AuthService.authorize

review graph path UserController.update AuthService.authorize

review graph tests AuthService.authorize
```

Without this, debugging incorrect reviews becomes extremely difficult.

---

# 85. Explainability

Every published finding should internally be traceable to:

```text
PR change
↓
symbol
↓
graph context
↓
reviewer
↓
candidate finding
↓
verification evidence
↓
published finding
```

An operator should be able to reconstruct why the system produced a comment.

---

# 86. Internal Review Trace

Example:

```text
Finding F-2847

Anchor:
src/auth/auth.service.ts:81

Changed symbol:
AuthService.authorize

Relevant path:
UserController.update
→ AdminService.updateUser
→ AuthService.authorize

Candidate generated by:
security-reviewer:v3

Verified by:
graph verifier
base/head comparison

Confidence:
0.93
```

Do not expose hidden model reasoning.

Expose evidence and traceable system facts.

---

# 87. Model Architecture

Models should sit behind a provider-neutral gateway.

```text
Review Task
    │
    ▼
Model Router
    │
    ├── low-cost model
    ├── high-reasoning model
    └── local model
```

Routing can consider:

```text
task complexity
risk
context length
repository policy
privacy requirements
cost budget
```

---

# 88. Model Responsibilities

Models should primarily handle:

- semantic interpretation,
- cross-code reasoning,
- change consequence analysis,
- architectural reasoning,
- candidate explanation,
- ambiguous defect reasoning.

Models should not be responsible for:

- parsing code,
- computing line diffs,
- discovering imports,
- calculating simple call graphs,
- detecting lint violations,
- counting references.

---

# 89. Model Input Contract

Every model call should use structured context.

Example:

```json
{
  "task": "correctness_review",
  "changedSymbols": [],
  "changeSummary": {},
  "graphNeighborhood": {},
  "tests": [],
  "repositoryRules": [],
  "riskSignals": [],
  "deterministicFindings": []
}
```

Avoid ad-hoc giant prompts assembled from raw files.

---

# 90. Review Budget Manager

The product should expose budgets for:

```text
maximum reviewed symbols
maximum graph expansion
maximum model tokens
maximum candidate findings
maximum model calls
maximum review latency
```

These may vary by plan or deployment.

---

# 91. Large PR Handling

Large PRs must degrade gracefully.

Instead of:

```text
PR too large → useless review
```

perform:

```text
PR
↓
change clustering
↓
risk ranking
↓
critical cluster review
↓
secondary cluster review if budget allows
```

A summary should transparently indicate any skipped low-risk regions.

---

# 92. Change Clustering

Symbols may be clustered by:

```text
module
dependency relationship
execution path
feature
API
database entity
semantic similarity
```

Each cluster can become an independent review unit.

---

# 93. Generated Code

Generated code should be detected through:

- known directories,
- generated headers,
- build configuration,
- file patterns,
- repository profile.

Generated changes are usually not LLM-reviewed directly.

The generator or input definition should be reviewed instead where possible.

---

# 94. Monorepo Support

Repository initialization must identify workspace boundaries.

Example:

```text
repo/
├── apps/api
├── apps/web
├── packages/auth
├── packages/db
└── packages/shared
```

The graph must support both:

```text
intra-package dependencies
```

and:

```text
cross-package dependencies
```

Review budgets may prioritize the changed workspace and impacted dependents.

---

# 95. Polyglot Support

The CodeGraph schema must remain language-neutral.

Each language adapter outputs a common intermediate representation.

```text
TypeScript Parser
        │
Python Parser
        │
Rust Parser
        │
Java Parser
        ▼
Common Symbol/Edge IR
        │
        ▼
CodeGraph
```

---

# 96. Initial Language Priority

Recommended implementation order:

```text
1. TypeScript / JavaScript
2. Python
3. Java
4. Go
5. Rust
```

The framework must not assume JavaScript semantics.

---

# 97. TypeScript Reference Analyzer

Given a NestJS reference repository as the initial proving ground, the first language adapter should support:

- ES modules,
- CommonJS where required,
- TypeScript types,
- classes,
- interfaces,
- decorators,
- NestJS modules/controllers/providers,
- TypeORM entities,
- BullMQ processors/producers,
- Jest tests,
- route decorators.

These capabilities belong to the TypeScript/NestJS adapter rather than the generic graph core.

---

# 98. Framework Adapters

Framework-aware extraction should be modular.

Examples:

```text
NestJS
Spring Boot
Django
FastAPI
Express
React
Next.js
Rails
```

Framework adapters enrich generic AST relationships.

For example NestJS:

```text
@Get('/users/:id')
```

should create:

```text
APIEndpoint
   → HANDLED_BY
UserController.getUser
```

---

# 99. Test Mapping

Tests should be mapped to production symbols using several signals:

```text
direct imports
method invocation
test naming
coverage data
mock references
path conventions
semantic similarity
```

Result:

```text
AuthService.authorize
        │
        ├── tested by authorize.spec.ts
        └── covered by AuthController.e2e-spec.ts
```

---

# 100. Runtime Data Integration

Future versions may optionally enrich the graph with:

```text
coverage
traces
profiling
production exceptions
```

Example:

```text
static call graph
+
observed runtime trace
```

This should remain optional.

---

# 101. Data Storage Model

The architecture should permit several backends.

Logical storage categories:

```text
metadata DB
graph store
blob/artifact store
cache
vector/semantic store
```

A first implementation does not require five separate technologies.

A pragmatic initial architecture can consolidate these.

---

# 102. Graph Storage Requirements

Must support:

```text
node lookup
neighbor traversal
reverse-edge traversal
path search
filtered traversal
batch upsert
versioned snapshots
incremental edge replacement
```

The abstraction should not tie the entire product to one graph database.

---

# 103. Relational Graph Storage

A practical initial implementation may use PostgreSQL.

Example:

```text
graph_nodes
graph_edges
symbols
repository_snapshots
```

Indexes:

```text
(repository_id, symbol_id)
(repository_id, source_node_id, edge_type)
(repository_id, target_node_id, edge_type)
(repository_id, file_id)
```

A dedicated graph database should only be introduced when query behavior demonstrates the need.

---

# 104. Snapshot Model

Snapshots should represent repository states.

```text
Repository
   │
   ├── snapshot base SHA
   ├── snapshot PR SHA
   └── snapshot latest default branch
```

Avoid complete graph duplication where possible.

Snapshots may use:

```text
base graph
+
delta
```

---

# 105. Repository Intelligence Versioning

Every derived artifact should identify:

```text
source commit
analyzer version
graph version
profile version
model version where applicable
```

This makes stale analysis detectable.

---

# 106. API Service

Illustrative API:

```http
POST /repositories
POST /repositories/:id/initialize

GET /repositories/:id/status
GET /repositories/:id/profile

POST /repositories/:id/graph/rebuild

POST /pull-requests/:id/review
GET  /pull-requests/:id/reviews/:reviewId

GET /reviews/:id/findings

POST /findings/:id/feedback
```

Internal APIs may expose graph queries separately.

---

# 107. Webhook Architecture

```text
Provider Webhook
      │
      ▼
Webhook Gateway
      │
      ▼
Signature Validation
      │
      ▼
Event Normalization
      │
      ▼
Idempotency Check
      │
      ▼
PR Review Orchestrator
```

Webhook acknowledgement should not wait for the complete review.

---

# 108. Review State Machine

Suggested lifecycle:

```text
RECEIVED
   ↓
INDEXING
   ↓
ANALYZING
   ↓
REVIEWING
   ↓
VERIFYING
   ↓
PUBLISHING
   ↓
COMPLETED
```

Failure states:

```text
FAILED_INDEXING
FAILED_ANALYSIS
FAILED_REVIEW
FAILED_PUBLISH
SUPERSEDED
CANCELLED
```

---

# 109. Partial Failure

One failed reviewer should not necessarily fail the entire review.

Example:

```text
security succeeded
correctness succeeded
performance failed
```

The review can complete with degraded coverage if policy permits.

The system should record the missing reviewer.

---

# 110. Security Requirements

Repository source code is sensitive data.

The system must provide:

- encryption in transit,
- encryption at rest,
- tenant isolation,
- least-privilege provider credentials,
- webhook signature verification,
- secret redaction,
- audit logging,
- configurable retention,
- model-provider data controls,
- access-controlled repository artifacts.

---

# 111. Secret Handling

Repository initialization should detect likely secrets and prevent accidental model transmission.

Model context should never intentionally contain:

- private keys,
- access tokens,
- passwords,
- `.env` values,
- credentials.

Where such values appear in source:

```text
KEY="<redacted>"
```

while retaining enough context to reason about the code pattern.

---

# 112. Tenant Isolation

Hosted architecture must scope all objects by:

```text
organization
repository
```

Every query touching repository data must enforce tenant ownership.

---

# 113. Source Retention

Deployments should support policies such as:

```text
retain source cache indefinitely
retain for N days
retain graph but not raw source
ephemeral source processing
```

Enterprise customers may require stricter policies.

---

# 114. Observability

Every review should emit structured traces.

Trace hierarchy:

```text
pr_review
├── repository_update
├── change_analysis
├── context_selection
├── reviewer.correctness
├── reviewer.security
├── reviewer.tests
├── candidate_aggregation
├── verification
└── publish
```

---

# 115. Metrics

Core metrics:

### Review quality

```text
published findings / candidate findings
developer acceptance rate
false-positive rate
dismissal rate
finding resolution rate
duplicate suppression rate
```

### Performance

```text
review latency
incremental index latency
context generation latency
model latency
verification latency
```

### Cost

```text
tokens per PR
model calls per PR
cache hit ratio
cost per reviewed PR
cost per published finding
```

### Graph health

```text
nodes
edges
incremental invalidations
parse failures
unresolved symbols
graph rebuild frequency
```

---

# 116. Review Quality KPI

The primary product KPI should not be:

```text
comments per PR
```

Instead optimize:

```text
actionable accepted findings
/
published findings
```

Secondary metric:

```text
important bugs found before merge
```

---

# 117. False Positive KPI

A core objective should be:

```text
false-positive rate < 10%
```

for medium/high-confidence comments after calibration.

Long-term target:

```text
< 5%
```

for high-confidence findings.

Exact definitions must be standardized.

---

# 118. Performance Targets

Initial targets for already-indexed repositories:

### Small PR

```text
< 10 changed files
target review completion: < 60 seconds
```

### Medium PR

```text
10–50 changed files
target: < 2 minutes
```

### Large PR

```text
50–200 changed files
target: < 5 minutes with risk-budgeting
```

These are product targets rather than absolute guarantees.

---

# 119. Incremental Index Target

When fewer than 10 files change:

```text
graph update should generally complete in seconds
```

The system should not perform work proportional to the full repository size unless dependency invalidation requires it.

---

# 120. Repository Scale Target

Initial production target:

```text
100,000 source files
1,000,000 symbols
multi-million-edge graph
```

The architecture should avoid assumptions that every graph query fits entirely in one prompt or one process.

---

# 121. Review Reproducibility

Given:

```text
same repository state
same review config
same analyzer versions
same model version
```

the review should be operationally reproducible.

LLM outputs may vary, but structural inputs and evidence must remain deterministic.

---

# 122. Review Configuration

Example:

```yaml
review:
  reviewers:
    correctness: true
    security: true
    tests: true
    performance: true
    architecture: true
    maintainability: false

  confidence:
    minimum_publish: 0.72

  budgets:
    max_symbols: 100
    max_context_tokens: 40000

  generated:
    ignore:
      - "**/*.generated.ts"

  risk:
    paths:
      "src/auth/**": critical
      "migrations/**": high
```

---

# 123. Repository Policy File

Optional repository-owned file:

```text
.review/config.yaml
```

This allows review behavior to remain version-controlled.

---

# 124. Suppression Mechanisms

Teams should be able to suppress:

```text
finding type
path
symbol
rule
specific comment fingerprint
```

Suppressions should be explicit and auditable.

---

# 125. Reference Consumer Repository

Existing prototype CodeGraph work should be treated as:

```text
reference implementation
+
first real-world testbed
```

not:

```text
core product schema
```

The relationship should become:

```text
Generic ReviewGraph
      │
      ├── Generic TypeScript Analyzer
      ├── Generic NestJS Adapter
      ├── Generic TypeORM Adapter
      └── Generic BullMQ Adapter
              │
              ▼
       Reference Repository
              │
              ▼
       Reference Profile
```

---

# 126. Consumer-Specific Intelligence

the reference consumer can then provide project-specific rules such as:

```text
account/client tenant scoping
RoleGuard expectations
ClientAccessGuard expectations
ResponseUtil conventions
queue architecture rules
migration-only DB changes
domain invariants
```

These should live in:

```text
repository profile
+
repository rules
```

not the generic graph implementation.

---

# 127. Extraction Strategy From the Prototype CodeGraph

Existing components should be classified into four categories.

## Category A — Generic and reusable

Examples:

```text
AST parsing
symbol identity
graph primitives
graph traversal
incremental invalidation
dependency analysis
```

Move directly into reusable packages.

---

## Category B — Generic concept, consumer implementation

Examples:

```text
NestJS route detection
TypeORM relation detection
BullMQ producer/consumer edges
```

Generalize behind framework adapters.

---

## Category C — consumer-only policy

Examples:

```text
domain filing invariants
account tenancy
specific queue naming rules
consumer module boundaries
```

Keep outside the core.

---

## Category D — Temporary implementation details

Anything existing merely because of the current CodeGraph prototype should be reassessed rather than automatically migrated.

---

# 128. Implementation Phases

## Phase 0 — Existing System Audit

Before implementation:

```text
inventory current prototype CodeGraph
map node types
map edge types
map parser logic
map storage
map graph queries
map cache
map architecture assumptions
identify consumer coupling
```

Deliverable:

```text
CodeGraph Extraction Report
```

---

# 129. Phase 1 — Generic Repository Intelligence Core

Build:

```text
repository model
language analyzer interface
common graph IR
node/edge schema
graph store
repository snapshots
incremental update engine
```

Scope:

```text
TypeScript only
local repositories
```

No AI reviewer required yet.

Success criterion:

```text
changing one file updates only affected graph regions correctly
```

---

# 130. Phase 2 — Semantic Diff Engine

Build:

```text
git diff ingestion
diff → AST mapping
changed symbol detection
symbol change classification
base/head comparison
```

Success criterion:

Given a PR, the engine can reliably answer:

```text
what symbols changed
how they changed
what dependencies changed
```

---

# 131. Phase 3 — Impact Graph

Build:

```text
caller/callee expansion
type relationship expansion
test mapping
API relationships
configuration relationships
risk classification
```

Success criterion:

For a changed symbol, relevant surrounding behavior can be retrieved without scanning the repository.

---

# 132. Phase 4 — Context Engine

Build:

```text
candidate retrieval
ranking
budgeting
compression
context packages
```

Success criterion:

High-relevance context can be produced for a reviewer under a defined token budget.

---

# 133. Phase 5 — First Reviewer

Implement only:

```text
Correctness Reviewer
```

Do not build six reviewers simultaneously.

Pipeline:

```text
change model
→ context
→ deterministic checks
→ correctness model
→ candidate findings
```

Success criterion:

Useful candidate findings appear on real reference-repository PRs.

---

# 134. Phase 6 — Verification Engine

Build:

```text
changed-code anchor verification
graph evidence verification
base/head comparison
contradiction checks
confidence
dedup
```

This phase should occur before broad reviewer expansion.

Success criterion:

False positives materially decrease.

---

# 135. Phase 7 — GitHub Integration

Build:

```text
GitHub App
webhooks
PR fetching
inline comments
review summary
superseding stale reviews
```

At this point the system becomes an end-to-end product.

---

# 136. Phase 8 — Additional Reviewers

Add sequentially:

```text
Security
Tests
Architecture
Performance
Maintainability
```

Each reviewer must prove incremental value.

---

# 137. Phase 9 — Repository Profile

Build convention inference and explicit repository policies.

Use the reference repository to validate:

```text
architecture conventions
queue patterns
error handling patterns
testing patterns
```

---

# 138. Phase 10 — Historical Intelligence

Add:

```text
previous PR ingestion
finding outcomes
repository review history
module regression signals
```

Historical signals should initially affect prioritization only.

---

# 139. Phase 11 — Multi-Provider Support

After core review quality is stable:

```text
GitLab
Bitbucket
```

Do not complicate the first architecture with provider-specific features.

---

# 140. MVP Definition

The MVP should include:

```text
TypeScript
NestJS-aware extraction
local + GitHub repositories
repository initialization
persistent graph
incremental graph updates
symbol-level diffs
caller/callee impact analysis
test mapping
correctness reviewer
security reviewer
verification
deduplication
inline comments
review summary
```

It should not yet require:

```text
full polyglot support
review-history learning
advanced dashboards
dedicated graph database
runtime tracing
automated fixes
```

---

# 141. MVP Acceptance Criteria

## Initialization

Given a supported TypeScript repository:

```bash
review init
```

must:

- identify source roots,
- detect TypeScript,
- detect package manager,
- detect tests,
- detect NestJS where applicable,
- produce persistent graph state,
- finish without manual graph configuration for normal projects.

---

## Incremental update

When one implementation file changes:

- unchanged files are not reparsed unnecessarily,
- removed symbols disappear,
- new symbols appear,
- relevant dependency edges update,
- dependent caches invalidate,
- graph results reflect the new commit.

---

## Diff mapping

For a modified method:

- the system identifies the method,
- records AST-level modifications,
- identifies changed calls,
- identifies affected API/test relationships.

---

## Impact analysis

For a changed symbol:

- direct callers can be retrieved,
- direct callees can be retrieved,
- relevant tests can be retrieved,
- API entry points can be identified where supported.

---

## Review

A PR review should:

- analyze changed symbols,
- retrieve bounded context,
- execute configured reviewers,
- generate candidate findings,
- verify findings,
- deduplicate findings,
- publish only candidates above threshold.

---

## Evidence

Every published finding must contain:

- changed-code anchor,
- concrete explanation,
- supporting evidence,
- relevant affected path where available.

---

# 142. Quality Evaluation Dataset

Create a benchmark repository corpus containing:

```text
known bugs
known safe changes
security regressions
test gaps
architecture violations
performance regressions
false-positive traps
```

For each PR:

```text
expected findings
forbidden findings
acceptable optional findings
```

This gives ReviewGraph a real evaluation harness.

---

# 143. Regression Harness

Every product change should run the review engine against benchmark PRs.

Track:

```text
precision
recall
false positives
latency
tokens
cost
```

An implementation change should not be considered an improvement simply because it produces more findings.

---

# 144. Precision vs Recall Strategy

For PR review, default bias should be toward **precision**.

Missing a low-confidence issue is preferable to repeatedly posting incorrect comments.

For example:

```text
security-critical repository
→ slightly higher recall

ordinary maintainability
→ very high precision threshold
```

---

# 145. Product Moat

The strongest defensible asset is not prompting.

It is the accumulated infrastructure around:

```text
persistent repository graphs
incremental updates
symbol identity
change models
context ranking
repository profiles
review verification
historical signal quality
```

Prompts and model providers will change.

Repository intelligence should remain valuable regardless of model generation.

---

# 146. Major Technical Risks

## Risk 1 — Incorrect call graph

Dynamic languages can make call relationships uncertain.

Mitigation:

```text
confidence-scored edges
framework enrichment
type information
runtime data later
```

---

## Risk 2 — Excessive graph complexity

Trying to model every semantic relationship can create an unusable graph.

Mitigation:

Start with relationships that directly improve review quality.

---

## Risk 3 — False convention inference

Repeated code is not necessarily a deliberate architectural rule.

Mitigation:

Use confidence, exceptions, explicit policy precedence, and human feedback.

---

## Risk 4 — Verification becomes another LLM prompt

If verification is only:

```text
"Are you sure?"
```

false-positive reduction will be weak.

Mitigation:

Verification must use different evidence, base/head comparison, graph facts, and deterministic checks.

---

## Risk 5 — Reviewer proliferation

Six reviewers may simply create six times more noise.

Mitigation:

Selective reviewer routing and strong aggregation.

---

## Risk 6 — Large repositories

Graph computation and semantic retrieval may become expensive.

Mitigation:

Incremental architecture, bounded traversal, caching, and workspace partitioning.

---

# 147. Critical Architectural Invariants

The following should be treated as hard architecture rules.

### Invariant 1

```text
PR review never requires a complete graph rebuild under normal operation.
```

### Invariant 2

```text
LLM output never directly becomes an external finding.
```

### Invariant 3

```text
Every published finding is anchored to changed behavior or impact created by the PR.
```

### Invariant 4

```text
Provider-specific code cannot leak into review-engine domain logic.
```

### Invariant 5

```text
consumer-specific rules cannot become generic graph semantics.
```

### Invariant 6

```text
Repository intelligence is versioned and reproducible.
```

### Invariant 7

```text
Context selection is explicitly budgeted.
```

### Invariant 8

```text
Graph inference uncertainty remains represented rather than hidden.
```

### Invariant 9

```text
Pre-existing unrelated defects are not presented as PR-introduced regressions.
```

### Invariant 10

```text
Comment quantity is never an optimization target.
```

---

# 148. Recommended Repository Structure

```text
reviewgraph/
├── apps/
│   ├── api/
│   ├── worker/
│   ├── cli/
│   └── dashboard/
│
├── packages/
│   ├── core/
│   ├── codegraph/
│   ├── languages/
│   ├── frameworks/
│   ├── diff/
│   ├── context/
│   ├── analysis/
│   ├── reviewers/
│   ├── verification/
│   ├── repository-profile/
│   ├── llm/
│   ├── integrations/
│   ├── persistence/
│   └── observability/
│
├── fixtures/
│   ├── repositories/
│   └── pull-requests/
│
├── benchmarks/
│
├── docs/
│   ├── architecture/
│   ├── graph-schema/
│   ├── analyzers/
│   ├── reviewers/
│   └── decisions/
│
└── tooling/
```

---

# 149. Key Domain Objects

Core entities:

```text
Repository

RepositorySnapshot

SourceFile

Symbol

GraphNode

GraphEdge

PullRequest

ChangedFile

ChangedSymbol

ChangeCluster

ImpactGraph

ContextPackage

ReviewRun

ReviewerRun

CandidateFinding

VerifiedFinding

PublishedFinding

RepositoryProfile

RepositoryConvention

ReviewFeedback
```

---

# 150. Finding Lifecycle

```text
GENERATED
   ↓
EVIDENCE_COLLECTED
   ↓
VERIFIED
   ↓
DEDUPLICATED
   ↓
PRIORITIZED
   ↓
PUBLISHED
```

Alternate states:

```text
SUPPRESSED_LOW_CONFIDENCE
SUPPRESSED_DUPLICATE
SUPPRESSED_PREEXISTING
SUPPRESSED_NOT_ACTIONABLE
SUPPRESSED_POLICY
INVALIDATED
```

Persisting suppression reasons is important for system evaluation.

---

# 151. Example Complete Review

Consider:

```diff
async authorize(user, resource) {
-  return this.permissionService.check(user.id, resource.id);
+  return user.role === 'admin';
}
```

## Stage 1 — Changed symbol

```text
AuthService.authorize()
```

## Stage 2 — Semantic changes

```text
removed dependency call:
PermissionService.check()

added:
direct role comparison

risk:
authorization behavior changed
```

## Stage 3 — Impact graph

```text
AuthService.authorize
  ← AdminService.updateUser
  ← UserController.update

AuthService.authorize
  ↔ AuthProvider.authorize

AuthService.authorize
  ← authorize.spec.ts
```

## Stage 4 — Security reviewer

Candidate:

```text
resource-level authorization appears removed
```

## Stage 5 — Verification

Check:

```text
Does caller perform equivalent permission validation?
→ no

Did base version use PermissionService?
→ yes

Does head version bypass it?
→ yes

Does affected path reach a public endpoint?
→ yes
```

## Stage 6 — Confidence

```text
0.94
```

## Stage 7 — Published comment

```text
🔴 High — Resource-level permission check removed

`authorize()` previously delegated to `PermissionService.check(user.id,
resource.id)`, but the new implementation only checks whether the user has
the `admin` role.

`UserController.update()` reaches this method through
`AdminService.updateUser()`, so resource-level permissions are no longer
evaluated on that mutation.

Evidence:
UserController.update()
  → AdminService.updateUser()
  → AuthService.authorize()

Preserve the existing resource permission check or enforce the equivalent
constraint before this path returns success.
```

This example represents the desired product behavior.

---

# 152. What Makes ReviewGraph Different

The important comparison is architectural.

A basic AI reviewer behaves approximately like:

```text
Diff
+
Repository Search
+
LLM
```

ReviewGraph should behave like:

```text
Persistent Program Model
+
Incremental Change Model
+
Impact Analysis
+
Targeted Retrieval
+
Specialized Reasoning
+
Verification
```

The graph is therefore not merely a retrieval optimization.

It is the system's model of **how the codebase behaves and relates**.

---

# 153. Final Product Principle

The architecture should optimize for:

```text
understand first
review second
verify third
comment last
```

not:

```text
read diff
generate opinions
```

The intended progression is:

```text
Repository
    ↓
Persistent Intelligence
    ↓
CodeGraph
    ↓
Changed Symbols
    ↓
Behavioral Change Model
    ↓
Impact Graph
    ↓
Risk-Aware Context
    ↓
Specialized Reviewers
    ↓
Deterministic + Semantic Verification
    ↓
High-Confidence Findings
```

That architecture creates the foundation for a reviewer that can eventually behave less like an AI commenting on code and more like an engineer who has already spent months understanding the repository.

---

# 154. Definition of Product Success

The product succeeds when developers perceive:

> **“When this reviewer comments, I should look at it.”**

rather than:

> **“The bot commented again.”**

Every architectural choice—from the persistent CodeGraph to incremental indexing, context ranking, reviewer specialization, base/head verification, confidence thresholds, and aggressive suppression—should be evaluated against that objective.

The competitive advantage is not generating more code-review text.

It is building enough repository intelligence to know **when there is something worth saying.**