# CI/CD: GitHub Actions → k3s Deployment

This guide walks through wiring up automatic Docker builds and zero-downtime deployments to your two-node k3s cluster every time you push to `main`.

---

## Overview

```
git push → GitHub Actions → ghcr.io (image) → kubectl apply → k3s
```

- Images are stored in **GitHub Container Registry (GHCR)** — free, no extra account needed.
- A dedicated **service account with minimal RBAC** (not cluster-admin) deploys to k3s.
- Two pods spread across both nodes via pod anti-affinity.
- Rolling updates ensure zero downtime on every deploy.

---

## Prerequisites

- SSH access to `static.190.25.233.167.clients.your-server.de` (control plane)
- Port **6443** open on the control plane firewall (k3s API server)
- `kubectl` installed locally (optional but useful for verification)

### Check port 6443 is reachable

```bash
# From your local machine
nc -zv static.190.25.233.167.clients.your-server.de 6443
```

If it times out, open the port:

```bash
# On the control plane server
ufw allow 6443/tcp
```

---

## Step 1 — Bootstrap the cluster (run once on the control plane)

SSH into the control plane and run all commands as root or with sudo.

### 1a. Verify both nodes are ready

```bash
kubectl get nodes
```

Expected output: two nodes both in `Ready` state.

### 1b. Create the namespace and RBAC for the deployer

```bash
kubectl apply -f k8s/namespace.yaml
kubectl apply -f k8s/rbac.yaml
```

This creates:
- Namespace `core-api`
- ServiceAccount `deployer` with a long-lived token secret
- Role limited to `deployments` and `services` in `core-api` only

### 1c. Build the deployer kubeconfig

```bash
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
```

### 1d. Base64-encode it — you'll paste this into GitHub

```bash
base64 -w 0 /tmp/deploy-kubeconfig.yaml
```

Copy the output (a single long string). You'll need it in Step 2.

---

## Step 2 — Add secrets to GitHub

Go to your repository → **Settings → Secrets and variables → Actions → New repository secret**.

| Secret name   | Value                                         |
|---------------|-----------------------------------------------|
| `KUBE_CONFIG` | The base64 string from Step 1d               |

`GITHUB_TOKEN` is automatic — no action needed.

---

## Step 3 — Enable GitHub Container Registry for the repo

1. Go to **Settings → Actions → General**
2. Under *Workflow permissions*, select **Read and write permissions**
3. Save

After the first successful workflow run, go to **Packages** on your profile, find `core_api`, and under **Package settings → Manage access** add your repository with **Write** access if it isn't already linked.

---

## Step 4 — First deploy (bootstrap manifests)

The workflow deploys `deployment.yaml` and `service.yaml` on every push. For the very first run, trigger it by pushing any change to `main`:

```bash
git add .
git commit -m "add k8s manifests and ci pipeline"
git push origin main
```

Watch the pipeline under **Actions** tab. Both jobs (`build` and `deploy`) should go green.

---

## Step 5 — Verify the deployment

```bash
# From the control plane
kubectl get pods -n core-api -o wide
```

Expected: two pods, one on each node, both `Running`.

```bash
kubectl get svc -n core-api
```

The `LoadBalancer` service gets a VIP assigned by k3s Klipper. Note the `EXTERNAL-IP`.

```bash
curl http://<EXTERNAL-IP>/version
# {"version":"0.1.0"}
```

---

## How deploys work going forward

1. Push any commit to `main`
2. GitHub Actions builds the Docker image tagged with the short commit SHA (e.g. `a3f9b12`)
3. Image is pushed to `ghcr.io/<your-org>/core_api:a3f9b12`
4. The deployment manifest has its image tag substituted in CI, then applied with `kubectl apply`
5. `kubectl rollout status` waits up to 2 minutes for pods to become healthy before the job succeeds
6. If the new pods fail readiness checks, the rollout stalls — old pods keep serving traffic

---

## Files created by this guide

| File | Purpose |
|------|---------|
| `k8s/namespace.yaml` | Namespace `core-api` |
| `k8s/rbac.yaml` | ServiceAccount + Role + RoleBinding for CI deployer |
| `k8s/deployment.yaml` | 2-replica deployment with anti-affinity, probes, resource limits |
| `k8s/service.yaml` | LoadBalancer service on port 80 → 3000 |
| `.github/workflows/deploy.yml` | Build + push + deploy pipeline |

---

## Troubleshooting

**Pipeline fails at `kubectl apply` with auth error**
- Re-check that `KUBE_CONFIG` secret is the base64 output from Step 1d (no newlines).

**`EXTERNAL-IP` stays `<pending>`**
- k3s Klipper LB requires the node to have a usable network interface. Run `kubectl describe svc core-api -n core-api` for events.

**Pod stuck in `ImagePullBackOff`**
- The GHCR package may be private. Go to the package settings and set visibility to **public**, or verify the workflow has `packages: write` permission.

**Only one pod is scheduled (both on same node)**
- Anti-affinity is `preferred`, not `required`, so this is allowed if one node is unhealthy. Check `kubectl get nodes`.
