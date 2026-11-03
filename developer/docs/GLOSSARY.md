# Home cluster

Vocabulary for the `home-cluster-1` platform: CI/CD, tenancy and access.

## Language

**Tenant**:
A GitHub owner (personal account or organization) admitted to run Pipelines-as-Code jobs in the cluster.
_Avoid_: customer, account, team

**Tenant namespace**:
The Kubernetes namespace allocated to one tenant; it is also where that tenant's Repository CRs and PipelineRuns live.
_Avoid_: org namespace, team namespace

**Run namespace**:
The namespace of the Repository CR that matched an event; PaC creates the PipelineRun there.
_Avoid_: target namespace (that is the per-PipelineRun override)

**Registry**:
The single declarative file listing tenants, their namespaces, tiers, repositories and developer emails.
_Avoid_: tenant list, teams file

**Tier**:
A tenant's capability level: `build` (no deploy credentials, no Kubernetes API egress) or `deploy` (API egress and deploy credentials for the operator's GitOps path). The tier is a property of the owner, not of a repository.
_Avoid_: plan, class, team type

**Tenant platform**:
The chart that renders one namespace and its platform objects per registry entry.
_Avoid_: tenancy chart, tenant chart

**Pruner**:
The post-install/post-upgrade Job that deletes managed tenant namespaces no longer in the registry and stale namespace-lister bindings.
_Avoid_: cleanup job, garbage collector

**Build tier**:
A tenant capability level with build/test workloads and tenant-supplied registry credentials, but no deploy credentials and no Kubernetes API access.
_Avoid_: untrusted tier, external tier

**Deploy tier**:
A tenant capability level for owners the cluster operator controls, with the credentials and API access needed for the GitOps deploy path.
_Avoid_: trusted tier, internal tier

**Viewer identity**:
The Kubernetes user a dashboard request is evaluated as (the Cognito email via impersonation).
_Avoid_: user, login

**Workload identity**:
The service account a PipelineRun's pods run as (`tekton-bot`), which determines what the pipeline code can do in the cluster.
_Avoid_: pipeline identity, bot user

**Installation**:
One GitHub App installation on a single owner; PaC mints a token per installation.
_Avoid_: app, integration

**Repository CR**:
The Pipelines-as-Code `Repository` object that registers one git repository; its namespace is the run namespace.
_Avoid_: repo config, pipeline config

## Service networking

**Edge gateway**:
The L7 entry point that terminates TLS and admits traffic from outside the cluster.
_Avoid_: load balancer (that is L4), ingress controller (an implementation)

**API gateway**:
The runtime that serves the composed GraphQL schema and resolves one query across multiple services.
_Avoid_: service mesh, GraphQL Mesh (the composer, not the runtime)

**Supergraph**:
The single GraphQL schema composed from participating service contracts.
_Avoid_: gateway config, schema registry

**Service contract**:
The API a service publishes for its callers: OpenAPI always, and GraphQL SDL when it opts into aggregation.
_Avoid_: endpoint, schema (ambiguous with a database schema)

**Federated subgraph**:
A service that publishes a GraphQL contract with entity keys and can be composed into the supergraph.
_Avoid_: GraphQL service, microservice

**Direct path**:
A service-to-service call that bypasses the API gateway and relies on mesh identity.
_Avoid_: internal API, sidecar

**Aggregation path**:
A call through the API gateway that resolves fields across multiple services.
_Avoid_: GraphQL call, fan-out

**Service mesh**:
The data plane that gives workloads cryptographic identity and secures service-to-service traffic.
_Avoid_: API gateway, ingress

**Public route**:
An edge route explicitly attached to the public gateway; without one, a service is not reachable from the internet.
_Avoid_: exposed service, ingress
