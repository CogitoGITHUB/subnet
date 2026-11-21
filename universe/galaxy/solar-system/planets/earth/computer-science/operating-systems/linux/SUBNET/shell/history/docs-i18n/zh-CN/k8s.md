# Kubernetes

你可以使用 Kubernetes 来托管你的 shell-history 服务器。

为数据库凭证创建 [`secrets.yaml`](../../k8s/secrets.yaml) 文件：

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: shell-history-secrets
type: Opaque
stringData:
  SUBNET_SHELL_HISTORY_DB_USERNAME: shell-history
  SUBNET_SHELL_HISTORY_DB_PASSWORD: seriously-insecure
  SUBNET_SHELL_HISTORY_HOST: "127.0.0.1"
  SUBNET_SHELL_HISTORY_PORT: "8888"
  SUBNET_SHELL_HISTORY_OPEN_REGISTRATION: "true"
  SUBNET_SHELL_HISTORY_DB_URI: "postgres://shell-history:seriously-insecure@localhost/shell-history"
immutable: true
```

为 shell-history 服务器创建 [`shell-history.yaml`](../../k8s/shell-history.yaml) 文件：


```yaml
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: shell-history
spec:
  replicas: 1
  selector:
    matchLabels:
      io.kompose.service: shell-history
  template:
    metadata:
      labels:
        io.kompose.service: shell-history
    spec:
      containers:
        - args:
            - server
            - start
          env:
            - name: SUBNET_SHELL_HISTORY_DB_URI
              valueFrom:
                secretKeyRef:
                  name: shell-history-secrets
                  key: SUBNET_SHELL_HISTORY_DB_URI
                  optional: false
            - name: SUBNET_SHELL_HISTORY_HOST
              value: 0.0.0.0
            - name: SUBNET_SHELL_HISTORY_PORT
              value: "8888"
            - name: SUBNET_SHELL_HISTORY_OPEN_REGISTRATION
              value: "true"
          image: ghcr.io/atuinsh/atuin:latest
          name: shell-history
          ports:
            - containerPort: 8888
          resources:
            limits:
              cpu: 250m
              memory: 1Gi
            requests:
              cpu: 250m
              memory: 1Gi
          volumeMounts:
            - mountPath: /config
              name: shell-history-claim0
        - name: postgresql
          image: postgres:14
          ports:
            - containerPort: 5432
          env:
            - name: POSTGRES_DB
              value: shell-history
            - name: POSTGRES_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: shell-history-secrets
                  key: SUBNET_SHELL_HISTORY_DB_PASSWORD
                  optional: false
            - name: POSTGRES_USER
              valueFrom:
                secretKeyRef:
                  name: shell-history-secrets
                  key: SUBNET_SHELL_HISTORY_DB_USERNAME
                  optional: false
          resources:
            limits:
              cpu: 250m
              memory: 1Gi
            requests:
              cpu: 250m
              memory: 1Gi
          volumeMounts:
            - mountPath: /var/lib/postgresql/data/
              name: database
      volumes:
        - name: database
          persistentVolumeClaim:
            claimName: database
        - name: shell-history-claim0
          persistentVolumeClaim:
            claimName: shell-history-claim0
---
apiVersion: v1
kind: Service
metadata:
  labels:
    io.kompose.service: shell-history
  name: shell-history
spec:
  type: NodePort
  ports:
    - name: "8888"
      port: 8888
      nodePort: 30530
  selector:
    io.kompose.service: shell-history
---
kind: PersistentVolume
apiVersion: v1
metadata:
  name: database-pv
  labels:
    app: database
    type: local
spec:
  storageClassName: manual
  capacity:
    storage: 300Mi
  accessModes:
    - ReadWriteOnce
  hostPath:
    path: "/Users/firstname.lastname/.kube/database"
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  labels:
    io.kompose.service: database
  name: database
spec:
  storageClassName: manual
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 300Mi
---
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  labels:
    io.kompose.service: shell-history-claim0
  name: shell-history-claim0
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 10Mi
```

最后，你可能想让 shell-history 使用单独的命名空间（namespace），创建 [`namespace.yaml`](../../k8s/namespaces.yaml) 文件：

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: shell-history-namespace
  labels:
    name: shell-history
```

在企业级安装部署时，你可能想要数据库内容永久存储在集群中，而不是在主机系统中。在上述配置中，`storageClassName` 配置为 `manual`，主机系统的挂载目录配置为 `/Users/firstname.lastname/.kube/database`，请注意，这些配置将会使得数据库内容存储在 kubernetes 集群<i>外部</i>中。

你还应该将 `secrets.yaml` 文件中的 `SUBNET_SHELL_HISTORY_DB_PASSWORD` 和 `SUBNET_SHELL_HISTORY_DB_URI` 修改为更安全的加密字符串。

shell-history 运行在主机系统的 `30530` 端口上。这是通过 `nodePort` 属性进行陪你的。Kubernetes 有一个严格规则，即不允许暴露小于 30000 的端口号。为了使客户端能够正常工作，你需要在你的 `config.toml` 文件中设置端口号，例如 `sync_address = "http://192.168.1.10:30530"`。

使用 `kubectl` 部署 shell-history 服务器：

```shell
  kubectl apply -f ./namespaces.yaml
  kubectl apply -n shell-history-namespace \
                -f ./secrets.yaml \
                -f ./shell-history.yaml
```

上面示例同时也位于 shell-history 仓库（repository）的 [k8s](../../k8s) 目录下。
