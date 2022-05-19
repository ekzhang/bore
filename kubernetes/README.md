# How to run bore on a Kubernetes cluster?

Having a Kubernetes cluster in the home network is not that uncommon nowadays. Running it behind NAT and exposing various services to the outer world however, comes with its own set of networking problems. Especially, when you can't (or don't want to) assign a fixed public IP address to the router and your ISP rotates the addresses among the customers regularly.

Typical use cases can be hosting your own website, a git server or you just simply managing the cluster from the outside. `bore` can perfectly handle these scenarios.

Since [v0.2.3](https://github.com/ekzhang/bore/releases/tag/v0.2.3) we can run `bore` in a Docker container. And if we can run something in a container, chances are we can run it on Kubernetes too.

The steps below assume some basic knowledge of Kubernetes concepts and expertise in working with manifest files, as well as using [kubectl](https://kubernetes.io/docs/reference/kubectl/). The instructions focus on `bore local` as `bore server` requires opening TCP ports dynamically, which is a hard sell on Kubernetes. You can always use the public `bore.pub` server or also can set your own server up at your favorite cloud provider without breaking the bank. I run mine at AWS, on a `t4g.nano` spot instance and the monthly cost is around 1 USD. The instructions won't cover setting up the cluster or the various hosted applications either.

# Example architecture



## Components

* `my.bore.server` - out in the wild, exposing TCP ports
    * `7835` - bore control port
    * `6443` - Kubernetes API server
    * `2222` - SSH
    * `443` - HTTPS
    * `80` - HTTP
* A single-node Kubernetes cluster running on a Linux machine
* `bore local` - deployment on the cluster and routing traffic
  * `6443` - to the API server
  * `2222` - to the node's SSH daemon
  * `80` and `443` - to the ingress controller

## bore setup

* Create a `Namespace`

  ```yaml
  kind: Namespace
  apiVersion: v1
  metadata:
    name: bore
  ```

* Create a `Secret` to store `BORE_SECRET`

  ```yaml
  kind: Secret
  apiVersion: v1
  metadata:
    name: bore-secrets
    namespace: bore
  type: Opaque
  data:
    BORE_SECRET: <base64 encoded secret>
  ```

* Create the `Deployment`. I's a multi-container deployment, each container runs a single tunnel. Create as many tunnels as you need.

  ```yaml
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
        - name: bore-http # container 1
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
            value: "192.168.1.1"
          - name: BORE_SECRET
            valueFrom:
              secretKeyRef:
                name: bore-secrets
                key: BORE_SECRET
          command: ["./bore"]
          args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "$(PORT)"]
        - name: bore-https # container 2
          ...
  ```

* Comments on the `Deployment`
  * 1 replica is all you need.
  * Set the deployment strategy to `Recreate`. What it does it kills the `Pod` first then creates the new one only afterwards. This will be important when restarting the `Pod` from a cron job. More about this later.
  * `bore` doesn't need root privileges, hence the `securityContext` settings.
  * The `REDIRECT_TO` environment variable contains the external IP of the Load Balancer of the Kubernetes node that the `pod` runs on. You'll find a bit more detailed explanation in the use cases section.

## Restarting bore

When your Internet Service Provider rotates your public IP address (mine does, every 24 hours) you'll loose connection to your home network because as of today `bore local` can't reconnect to the server. In this case the `Pod` has to be restarted. The workaround below runs as a Kubernetes `CronJob`. It checks for the router's public IP address. If it differs from the stored value in a `ConfigMap` then the job assumes that the IP address was rotated, so it restarts the `Pod` and updates the `ConfigMap` with the new value. If the deployment strategy is not `Recreate` then the Kubernetes scheduler tries to start the new `Pod` first and shuts the old down only afterwards. But in this case the old `Pod` still uses the TCP ports therefore the new `Pod` will never come alive.

* Create a `ConfigMap` with your current public IP address

  ```yaml
  apiVersion: v1
  kind: ConfigMap
  metadata: 
    name: bore-local-external-ip
    namespace: bore
  data:
    external-ip: 1.1.1.1
  ```

* Create a `ServiceAccount` that the job will use

  ```yaml
  kind: ServiceAccount
  apiVersion: v1
  metadata:
    name: restart-bore-pods
    namespace: bore
  ```

* Create a `Role` that can restart the `Pod` and update the `ConfigMap` in the namespace

  ```yaml
  apiVersion: rbac.authorization.k8s.io/v1
  kind: Role
  metadata:
    name: restart-bore-pods
    namespace: bore
  rules:
    - apiGroups: [""]
      resources: ["pods"]
      verbs: ["get", "patch", "list", "watch", "delete"]
    - apiGroups: [""]
      resources: ["configmaps"]
      verbs: ["*"]
  ```

* Bind the `Role` to the `ServiceAccount`

  ```yaml
  kind: RoleBinding
  apiVersion: rbac.authorization.k8s.io/v1
  metadata:
    name: restart-bore-pods
    namespace: bore
  roleRef:
    kind: Role
    name: restart-bore-pods
    apiGroup: rbac.authorization.k8s.io
  subjects:
    - kind: ServiceAccount
      name: restart-bore-pods
      namespace: bore
  ```

* Create another `ConfigMap` that contains the restart script and will be mounted into the `CronJob` as `/script/restart-bore-pods.sh`

  ```yaml
  kind: ConfigMap
  apiVersion: v1
  metadata:
    name: restart-bore-pods
    namespace: bore
  data:
    restart-bore-pods.sh: |
      #!/bin/bash

      NAMESPACE=bore
      CM=bore-local-external-ip
      BORE_POD_NAME_PATTERN="^bore-.*$"

      echo "getting my external IP..."
      MY_EXT_IP=$(curl -s ipinfo.io/ip 2>/dev/null)

      if [[ -z "${MY_EXT_IP}" ]] 
      then
          echo "error getting external IP"
          exit 0
      else
          echo "external IP: ${MY_EXT_IP}"
      fi

      echo "getting my stored external IP from ConfigMap..."
      STORED_EXT_IP=$(kubectl get configmap $CM -n $NAMESPACE -o jsonpath='{.data.external-ip}')

      if [[ -z "${STORED_EXT_IP}" ]]
      then
          echo "error getting stored IP"
          exit 0
      else
          echo "stored IP: ${STORED_EXT_IP}"
      fi

      if [[ ${MY_EXT_IP} != ${STORED_EXT_IP} ]]
      then
          echo "IPs don't match, restarting bore Pods..."
          for `POD` in $(kubectl get `pod` -n $NAMESPACE --no-headers=true | awk -F" " '{print $1}' | xargs )
          do  
              if [[ $`POD` =~ $BORE_POD_NAME_PATTERN ]]
              then
                kubectl delete `pod` $`POD` -n $NAMESPACE
              fi
          done
          echo "updating ConfigMap..."
          kubectl create configmap $CM -n $NAMESPACE --from-literal=external-ip=$MY_EXT_IP -o yaml --dry-run=client | kubectl replace -f -
      else
          echo "they're identical... nothing to do"
      fi
  ```

* Deploy the `CronJob`

  ```yaml
  kind: CronJob
  apiVersion: batch/v1
  metadata:
    name: restart-bore-pods
    namespace: bore
  spec:
    concurrencyPolicy: Forbid
    schedule: "*/5 * * * *"
    successfulJobsHistoryLimit: 1
    failedJobsHistoryLimit: 1
    jobTemplate:
      spec:
        activeDeadlineSeconds: 600
        backoffLimit: 1
        completions: 1
        template:
          spec:
            serviceAccountName: restart-bore-pods 
            restartPolicy: Never
            containers:
              - name: kubectl
                image: zkalmar/kubectl:latest
                imagePullPolicy: IfNotPresent
                command: ["/bin/bash"]
                args: ["/script/restart-bore-pods.sh"]
                volumeMounts:
                  - name: script
                    mountPath: /script
            volumes:
              - name: script
                configMap:
                  name: restart-bore-pods
                  defaultMode: 0777
  ```

  * Comments on the `CronJob`
    * Pick your schedule. I found the 5 mins checking interval good enough.
    * You can use any image for the conatiner. The restart script only needs `bash`, `curl` and `kubectl`.
    * Use the `BORE_POD_NAME_PATTERN` variable in the script to define which `Pod` to kill. This way you can avoid the job killing itself and being instantly recreated by the cluster scheduler, thus getting into an infinite loop.

## Use cases

### Access a website hosted on the cluster

This setup assumes that you have an Ingress Controller with a Load Balancer. It exposes ports `80` and `443` on the Load Balancer. The Ingress controller routes the traffic to the website `Service` and to the `Pod` behind.
You'll need 2 containers in the `bore` `Deployment` with different port settings:
* HTTP
  * `$PORT: 80`
  * `REDIRECT_TO: <Load Balancer IP>`
  * `args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "$(PORT)"]`
* HTTPS
  * `$PORT: 443`
  * `REDIRECT_TO: <Load Balancer IP>`
  * `args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "$(PORT)"]`

### Manage the cluster from outside

To manage a cluster either with [kubectl](https://kubernetes.io/docs/reference/kubectl/) or with a dashboard application like [k9s](https://k9scli.io) or [Lens](https://k8slens.dev) you need axcess to the API server. The Kubernetes API server is accessible on port `6443`. This traffic doesn't go through the Ingress Controller, so it has to be redirected to the IP address of the master node.
  * `$PORT: 6443`
  * `REDIRECT_TO: <Master Node IP>`
  * `args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "$(PORT)"]`
  
### SSH into a Kubernetes node

The `bore server` probably has its own SSH daemon running, hence port 22 is most likely already taken. Therefore pick another available port for the tunnel that you can redirect to the node's SSH port. The flow is something like this:
  `bore server:2222` -> `bore local:2222` -> `node:22`

  * `$PORT: 22`
  * `REDIRECT_TO: <Node IP>`
  * `args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "2222"]`
  
### Use your own Git server hosted on the cluster

This is a rather unusual scenario because a Git server like [Gitea](https://gitea.io/en-us/) exposes its own SSH port via its `Service`. And that port is not available through the Ingress Controller nor it's exposed on the node by default. The Ingress Controller can't do this so you'll have to find a way to make the `Service` port available on the Node. The answer is `NodePort` which comes with limitations.
  * The exposed port has to be higher than `30000`
  * The Git server `Pod` behind the `Service` will only be available on a single node. This is not a problem in case of a single-node cluster. If you have a multi-node cluster however, then you have to make sure that the `Pod` will always start on the same node otherwise you `bore` tunnel might not work after a restart (because the nodes have different IP addresses). So you'll have to "stick" the Git server `Pod` to the node using [NodeAffinity](https://kubernetes.io/docs/tasks/configure-pod-container/assign-pods-nodes-using-node-affinity/).

* Configure the `Service` with a `NodePort`
  
  ```yaml
  apiVersion: v1
  kind: Service
  metadata:
    name: gitea-ssh
    namespace: gitea
  spec:
    type: NodePort
    ports:
    - name: ssh
      port: 22
      targetPort: 22
      nodePort: 32222
      protocol: TCP
    selector:
      app: gitea
  ```
* And set up the `bore` tunnel accordingly
  
  * `$PORT: 32222`
  * `REDIRECT_TO: <Node IP>`
  * `args: ["local", "$(PORT)", "-l", "$(REDIRECT_TO)", "-s", "$(BORE_SECRET)", "-t", "$(SERVER)", "-p", "$(PORT)"]`