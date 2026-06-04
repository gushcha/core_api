# Deploying a Rust API to Kubernetes — A Complete Tutorial

This tutorial documents the full journey of taking a minimal Rust web service from source code
to a production-style deployment on a self-hosted two-node k3s cluster, with automatic builds
and deployments triggered by every `git push`.

---

## What We Built

```
git push
  └─► GitHub Actions
        ├─► builds Docker image
        ├─► pushes to GitHub Container Registry (ghcr.io)
        └─► kubectl apply ──► k3s cluster
                                ├─► node 1 (control plane)  167.233.25.190
                                └─► node 2 (worker)         167.233.24.111
                                        ↑
                                    Traefik (port 80)
                                    routes to 2 pods
```

Every commit to `main` produces a fresh Docker image tagged with the short git SHA,
deploys it to Kubernetes with a rolling update (zero downtime), and waits for both
pods to pass health checks before declaring success.

---

## The Stack Explained

### Rust + Axum

The application is a small HTTP API written in Rust using the
[Axum](https://github.com/tokio-rs/axum) framework and the
[Tokio](https://tokio.rs/) async runtime. It exposes one endpoint:

```
GET /version  →  {"version": "0.1.0"}
```

Rust was chosen because it compiles to a single static binary with no runtime
dependencies, which makes for very small and fast Docker images.

### Docker — multi-stage build

We use a [multi-stage Dockerfile](https://docs.docker.com/build/building/multi-stage/)
to keep the final image small:

- **Stage 1 (builder)** — uses the official `rust:1.87-slim` image to compile the binary.
  Dependencies are compiled in a separate layer so they are cached between builds
  and only the changed application code is recompiled on subsequent runs.
- **Stage 2 (runtime)** — uses `ubuntu:24.04` (matching the OS of the k3s nodes)
  and copies only the compiled binary. The result is roughly 100 MB instead of
  the ~1.4 GB builder image.

Further reading: [Docker multi-stage builds](https://docs.docker.com/build/building/multi-stage/)

### GitHub Container Registry (GHCR)

Images are stored in [GitHub Container Registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry)
(`ghcr.io`), which is free for public repositories and tightly integrated with
GitHub Actions — no extra credentials needed beyond `GITHUB_TOKEN`, which is
automatically provided to every workflow.

Each image is tagged with the short git SHA (e.g. `a3f9b12`) so every deployment
is traceable back to an exact commit. A `latest` tag is also pushed for convenience.

### GitHub Actions

[GitHub Actions](https://docs.github.com/en/actions) is the CI/CD platform built
into GitHub. Our workflow (`.github/workflows/deploy.yml`) has two jobs:

1. **build** — checks out code, logs into GHCR, builds the Docker image with
   [BuildKit](https://docs.docker.com/build/buildkit/) layer caching, and pushes it.
2. **deploy** — installs `kubectl`, substitutes the image tag into the deployment
   manifest, applies it to the cluster, and waits for the rollout to complete.

Further reading: [GitHub Actions documentation](https://docs.github.com/en/actions)

### k3s

[k3s](https://k3s.io/) is a lightweight, production-ready Kubernetes distribution
from Rancher. It packages the full Kubernetes control plane into a single binary
under 100 MB, making it practical to run on small VPS hosts like Hetzner Cloud.

Our cluster has two nodes:
- **Control plane** — `static.190.25.233.167.clients.your-server.de` — runs the
  Kubernetes API server, scheduler, and controller manager alongside workloads.
- **Worker** — `static.111.24.233.167.clients.your-server.de` — runs workloads only.

Further reading: [k3s documentation](https://docs.k3s.io/)

### Traefik

k3s ships with [Traefik](https://traefik.io/traefik/) as its default ingress
controller. Traefik runs as a DaemonSet on every node and owns ports 80 and 443.
It watches Kubernetes `Ingress` resources and automatically routes incoming HTTP
requests to the correct service.

This is the right way to expose HTTP services in k3s — rather than trying to bind
directly to port 80 per service (which causes conflicts), every service routes
through Traefik via an `Ingress` rule.

Further reading: [Traefik Kubernetes Ingress](https://doc.traefik.io/traefik/providers/kubernetes-ingress/)

---

## Kubernetes Concepts Used

### Deployment

A [`Deployment`](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)
tells Kubernetes how many copies of a pod to run and how to update them. Ours uses:

- **2 replicas** — one pod per node (enforced by pod anti-affinity below).
- **RollingUpdate** with `maxUnavailable: 0` — new pods must become healthy before
  old ones are terminated, ensuring zero downtime.
- **Resource requests and limits** — `requests` are used by the scheduler to place
  pods; `limits` cap what they can actually consume. Without these, a runaway process
  can starve the node.
- **Readiness and liveness probes** — Kubernetes calls `GET /version` to determine
  if a pod is ready to receive traffic (readiness) and if it should be restarted
  (liveness). Without these, traffic could be routed to a pod that has not started yet.
- **Pod anti-affinity** — a scheduling preference that spreads pods across different
  nodes (`topologyKey: kubernetes.io/hostname`). This means one node going down does
  not take the entire service with it.

Further reading: [Kubernetes Deployments](https://kubernetes.io/docs/concepts/workloads/controllers/deployment/)

### Service

A [`Service`](https://kubernetes.io/docs/concepts/services-networking/service/)
gives pods a stable internal DNS name and IP address. We use type `ClusterIP` —
the service is only reachable inside the cluster. External traffic enters through
Traefik, which forwards it to this service.

### Ingress

An [`Ingress`](https://kubernetes.io/docs/concepts/services-networking/ingress/)
defines HTTP routing rules. Our rule catches all paths (`/`) and forwards them to
the `core-api` service on port 3000. Traefik reads this rule and does the actual
proxying.

### RBAC

[Role-Based Access Control](https://kubernetes.io/docs/reference/access-authn-authz/rbac/)
limits what the GitHub Actions deployer can do. Instead of giving it cluster-admin
access (full control over everything), we created:

- A `ServiceAccount` named `deployer` in the `core-api` namespace.
- A `Role` scoped to `core-api` that only permits `get/list/create/update/patch`
  on `Deployments` and `Services`. It cannot touch other namespaces or cluster resources.
- A `RoleBinding` that grants the role to the service account.

This follows the [principle of least privilege](https://en.wikipedia.org/wiki/Principle_of_least_privilege):
a compromised CI token can only affect the `core-api` namespace.

---

## Problems We Hit and How We Solved Them

### 1. TLS certificate does not cover the external hostname

**Error:**
```
x509: certificate is valid for kubernetes, kubernetes.default, localhost,
ubuntu-4gb-fsn1-default-1, not static.190.25.233.167.clients.your-server.de
```

**Why it happened:** k3s generates a self-signed TLS certificate for its API server
on first boot. By default it only includes internal names. GitHub Actions connects
using the public DNS name, which was not in the certificate's
[Subject Alternative Names (SAN)](https://en.wikipedia.org/wiki/Subject_Alternative_Name).

**Fix:** Add the external hostname to k3s's `config.yaml` as a `tls-san` entry,
delete the old dynamic cert, and restart k3s so it regenerates the certificate
including the new SAN.

```bash
cat >> /etc/rancher/k3s/config.yaml << 'EOF'
tls-san:
  - static.190.25.233.167.clients.your-server.de
EOF
rm -f /var/lib/rancher/k3s/server/tls/dynamic-cert.json
systemctl restart k3s
```

When running `kubectl` directly on the server, use the local kubeconfig
(`/etc/rancher/k3s/k3s.yaml`) which connects via `127.0.0.1` and is always valid:

```bash
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
```

### 2. LoadBalancer service stuck in `<pending>`

**Symptom:** `kubectl get svc` showed `EXTERNAL-IP: <pending>` indefinitely.

**Why it happened:** We initially used `type: LoadBalancer` expecting k3s's built-in
[Klipper LB](https://github.com/k3s-io/klipper-lb) to assign an IP. Klipper works
by running a pod on each node that binds directly to port 80. However, Traefik was
already listening on port 80, so the Klipper pods could not start — they were in
`Pending` state competing for a port that was taken.

**Fix:** Switch to `type: ClusterIP` (internal only) and expose the service externally
through Traefik using an `Ingress` resource. This is the correct pattern for k3s:
Traefik owns the external ports, all services use ClusterIP and register Ingress rules.

Further reading: [k3s networking](https://docs.k3s.io/networking/basic-network-options)

### 3. Traefik returns 404 after switching to Ingress

**Symptom:** `curl http://<hostname>/version` returned Traefik's `404 page not found`.

**Why it happened:** Traefik received the request (so routing was working) but had
no matching rule. The `Ingress` resource had not been applied yet. After applying it,
Traefik still ignored it because newer versions of Traefik in k3s require an explicit
`ingressClassName: traefik` field — without it, Traefik does not claim the resource.

**Fix:** Add `ingressClassName: traefik` to the Ingress spec and re-apply.

---

## Step-by-Step Setup (from scratch)

### Step 0 — Fix the k3s TLS certificate

Run on the **control plane** as root:

```bash
mkdir -p /etc/rancher/k3s
cat >> /etc/rancher/k3s/config.yaml << 'EOF'
tls-san:
  - static.190.25.233.167.clients.your-server.de
EOF

rm -f /var/lib/rancher/k3s/server/tls/dynamic-cert.json
systemctl restart k3s

# Wait ~10 seconds, then verify
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml
kubectl get nodes
```

### Step 1 — Bootstrap cluster resources

Clone the repository and apply the base manifests:

```bash
git clone https://github.com/gushcha/core_api.git /opt/core_api
cd /opt/core_api
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml

kubectl apply -f k8s/namespace.yaml
kubectl apply -f k8s/rbac.yaml
kubectl apply -f k8s/service.yaml
kubectl apply -f k8s/ingress.yaml
```

Verify both nodes are ready:

```bash
kubectl get nodes
kubectl get ingress -n core-api
```

The ingress should show both node IPs in the `ADDRESS` column within a few seconds.

### Step 2 — Build the deployer kubeconfig for GitHub Actions

GitHub Actions needs credentials to talk to the k3s API from outside the cluster.
We generate a kubeconfig that uses the service account token we created:

```bash
export KUBECONFIG=/etc/rancher/k3s/k3s.yaml

TOKEN=$(kubectl get secret deploy-token -n core-api \
  -o jsonpath='{.data.token}' | base64 -d)

CA=$(kubectl get secret deploy-token -n core-api \
  -o jsonpath='{.data.ca\.crt}')

cat > /tmp/deploy-kubeconfig.yaml << EOF
apiVersion: v1
kind: Config
clusters:
- name: k3s
  cluster:
    certificate-authority-data: ${CA}
    server: https://static.190.25.233.167.clients.your-server.de:6443
contexts:
- name: deploy
  context:
    cluster: k3s
    namespace: core-api
    user: deployer
current-context: deploy
users:
- name: deployer
  user:
    token: ${TOKEN}
EOF

# Base64-encode it — copy this output
base64 -w 0 /tmp/deploy-kubeconfig.yaml
```

### Step 3 — Add the secret to GitHub

1. Go to your repository → **Settings → Secrets and variables → Actions → New repository secret**
2. Name: `KUBE_CONFIG`, value: the base64 string from Step 2

### Step 4 — Enable write permissions for GitHub Actions

1. Go to **Settings → Actions → General**
2. Under *Workflow permissions* select **Read and write permissions**
3. Save

This allows the workflow to push images to GitHub Container Registry using
the automatic `GITHUB_TOKEN`.

### Step 5 — Trigger the first deployment

```bash
git add .
git commit -m "add k8s manifests and ci pipeline"
git push origin main
```

Watch the **Actions** tab in GitHub. Both the `build` and `deploy` jobs should
go green within a couple of minutes.

### Step 6 — Verify

```bash
curl http://static.190.25.233.167.clients.your-server.de/version
# {"version":"0.1.0"}
```

Check that both pods are running, one per node:

```bash
kubectl get pods -n core-api -o wide
```

---

## Files in This Repository

| File | Purpose |
|------|---------|
| `Dockerfile` | Multi-stage build: compile in `rust:slim`, run in `ubuntu:24.04` |
| `.dockerignore` | Excludes `target/` and `.git/` from the build context |
| `k8s/namespace.yaml` | Creates the `core-api` namespace |
| `k8s/rbac.yaml` | ServiceAccount + Role (least-privilege) + RoleBinding for CI |
| `k8s/deployment.yaml` | 2 replicas, rolling update, probes, resource limits, anti-affinity |
| `k8s/service.yaml` | ClusterIP service — internal only, Traefik routes to it |
| `k8s/ingress.yaml` | Traefik Ingress rule — routes `/*` on port 80 to the service |
| `.github/workflows/deploy.yml` | CI/CD pipeline: build → push → deploy |

---

## Further Reading

- [Kubernetes concepts](https://kubernetes.io/docs/concepts/) — official docs covering all resources used here
- [k3s documentation](https://docs.k3s.io/) — k3s-specific configuration and architecture
- [Traefik Kubernetes Ingress](https://doc.traefik.io/traefik/providers/kubernetes-ingress/) — routing rules, TLS, middlewares
- [GitHub Actions](https://docs.github.com/en/actions) — workflow syntax, secrets, permissions
- [GitHub Container Registry](https://docs.github.com/en/packages/working-with-a-github-packages-registry/working-with-the-container-registry) — image storage and access control
- [Axum web framework](https://docs.rs/axum/latest/axum/) — Rust HTTP framework used in the API
- [Docker multi-stage builds](https://docs.docker.com/build/building/multi-stage/) — keeping images small
- [RBAC in Kubernetes](https://kubernetes.io/docs/reference/access-authn-authz/rbac/) — roles, bindings, least privilege
