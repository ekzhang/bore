# How to run bore on a Kubernetes cluster?

Having a Kubernetes cluster in the home network is not that uncommon nowadays. Running it behind NAT and exposing various services to the outer world however comes with its own set of networking problems. Especially when you can't (or don't want to) assign a fixed public IP address to the router and your ISP rotates the addresses among the customers regularly.

Typical use cases can be hosting your own website, a git server or you just simply managing the cluster from the outside. `bore` can perfectly handle these scenarios.

Since [v0.2.3](https://github.com/ekzhang/bore/releases/tag/v0.2.3) we can run `bore` in a Docker container. And if we can run something in a container, chances are we can run it on Kubernetes too.

The steps below assume some basic knowledge of Kubernetes concepts and expertise in working with manifest files, as well as using [kubectl](https://kubernetes.io/docs/reference/kubectl/). The instructions focus on `bore local` as `bore server` requires opening TCP ports dynamically, which is a hard sell on Kubernetes. You can always use the public `bore.pub` server or also can set your own server up at your favorite cloud provider without breaking the bank. I run mine at AWS on a `t4g.nano` spot instance and the monthly cost is around 1 USD.

# Example architecture



## Components

* `my.bore.server` - out in the wild, exposing TCP ports
    * 7835 - bore control port
    * 6443 - Kubernetes API server
    * 2222 - SSH
    * 443 - HTTPS
    * 80 - HTTP
* A single node Kubernetes cluster running on a Linux machine
* `bore local` - deployment on the cluster and routing traffic
  * 6443 - to the API server
  * 2222 - to the node's SSH daemon
  * 80 and 443 - to the ingress controller

The instructions don't cover setting up `bore server`, the cluster or the various hosted applications.

## bore setup

* Create a namespace
  ```
  kind: Namespace
  apiVersion: v1
  metadata:
    name: bore
  ```
* Create a Kubernetes secret for the shared `$BORE_SECRET`
  ```
  kind: Secret
  apiVersion: v1
  metadata:
    name: bore-secrets
    namespace: bore
  type: Opaque
  data:
    BORE_SECRET: <base64 encoded secret>
  ```
* Create the bore deployment. I's a multi-container deployment, each container runs a tunnel
  ```
  apiVersion: apps/v1
  kind: Deployment
  metadata:
    name: bore-local
    namespace: bore
  spec:
    selector:
      matchLabels:
        app: bore-tunnels
    replicas: 1
    strategy:
      type: Recreate
    template:
      metadata:
        labels:
          app: bore-tunnels
      spec:
        containers:
        - name: bore-http
          image: ekzhang/bore:latest
          imagePullPolicy: IfNotPresent
          securityContext:
            runAsUser: 1000
            runAsGroup: 1000
          env:
          - name: SERVER
            value: "my.bore.server"
          - name: PORT
            value: "80"
          - name: REDIRECT_TO
            value: "192.168.1.1"  # the IP address of the Kubernetes node
          - name: BORE_SECRET
            valueFrom:
              secretKeyRef:
                name: bore-secrets
                key: BORE_SECRET
          command: ["./bore"]
          args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "$(PORT)"]
        - name: bore-https
          ...
  ```
  

## Use cases

1. Access a website hosted on the cluster
2. Manage the cluster from outside
4. SSH into the Kubernetes node
5. Use your own git server hosted on the cluster
