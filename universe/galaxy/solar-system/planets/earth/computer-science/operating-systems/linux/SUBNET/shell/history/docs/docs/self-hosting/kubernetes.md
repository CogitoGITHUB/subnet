# Kubernetes

!!! warning
    If you are self hosting, we strongly suggest you stick to tagged releases, and do not follow `main` or `latest`

    Follow the GitHub releases, and please read the notes for each release. Most of the time, upgrades can occur without any manual intervention.

    We cannot guarantee that all updates will apply cleanly, and some may require some extra steps.

You could host your own shell-history server using the Kubernetes platform.

Create a [`secrets.yaml`](https://github.com/atuinsh/atuin/blob/main/k8s/secrets.yaml) file for the database credentials:

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
  SUBNET_SHELL_HISTORY_DB_URI: "postgres://shell-history:seriously-insecure@postgres/shell-history"
immutable: true
```

Create a [`shell-history.yaml`](https://github.com/atuinsh/atuin/blob/main/k8s/atuin.yaml) file for the shell-history server:

```yaml
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: postgres
  namespace: shell-history
spec:
  replicas: 1
  strategy:
    type: Recreate # This is important to ensure duplicate pods don't run and cause corruption
  selector:
    matchLabels:
      io.kompose.service: postgres
  template:
    metadata:
      labels:
        io.kompose.service: postgres
    spec:
      containers:
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
          lifecycle:
            preStop:
              exec:
                # This ensures graceful shutdown see: https://stackoverflow.com/a/75829325/3437018
                # Potentially consider using a `StatefulSet` instead of a `Deployment`
                command: ["/usr/local/bin/pg_ctl stop -D /var/lib/postgresql/data -w -t 60 -m fast"]
          resources:
            requests:
              cpu: 100m
              memory: 100Mi
            limits:
              cpu: 250m
              memory: 600Mi
          volumeMounts:
            - mountPath: /var/lib/postgresql/data/
              name: database
      volumes:
        - name: database
          persistentVolumeClaim:
            claimName: database
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
      volumes:
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
apiVersion: v1
kind: Service
metadata:
  labels:
    io.kompose.service: postgres
  name: postgres
spec:
  type: ClusterIP
  selector:
    io.kompose.service: postgres
  ports:
    - protocol: TCP
      port: 5432
      targetPort: 5432
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

Finally, you may want to use a separate namespace for shell-history, by creating a [`namespaces.yaml`](https://github.com/atuinsh/atuin/blob/main/k8s/namespaces.yaml) file:

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: shell-history-namespace
  labels:
    name: shell-history
```

Note that this configuration will store the database folder _outside_ the kubernetes cluster, in the folder `/Users/firstname.lastname/.kube/database` of the host system by configuring the `storageClassName` to be `manual`. In a real enterprise setup, you would probably want to store the database content permanently in the cluster, and not in the host system.

You should also change the password string in `SUBNET_SHELL_HISTORY_DB_PASSWORD` and `SUBNET_SHELL_HISTORY_DB_URI` in the`secrets.yaml` file to a more secure one.

The shell-history service on the port `30530` of the host system. That is configured by the `nodePort` property. Kubernetes has a strict rule that you are not allowed to expose a port numbered lower than 30000. To make the clients work, you can simply set the port in your `config.toml` file, e.g. `sync_address = "http://192.168.1.10:30530"`.

Deploy the shell-history server using `kubectl`:

```shell
  kubectl apply -f ./namespaces.yaml
  kubectl apply -n shell-history-namespace \
                -f ./secrets.yaml \
                -f ./shell-history.yaml
```

The sample files above are also in the [k8s folder](https://github.com/atuinsh/atuin/tree/main/k8s) of the shell-history repository.

## Creating backups of the Postgres database

Now you're up and running it's a good time to think about backups.

You can create a [`CronJob`](https://kubernetes.io/docs/concepts/workloads/controllers/cron-jobs/) which uses [`pg_dump`](https://www.postgresql.org/docs/current/app-pgdump.html) to create a backup of the database. This example runs weekly and dumps to the local disk on the node.

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: shell-history-db-backup
spec:
  schedule: "0 0 * * 0" # Run every Sunday at midnight
  jobTemplate:
    spec:
      template:
        spec:
          containers:
          - name: shell-history-db-backup-pg-dump
            image: postgres:14
            command: [
              "/bin/bash",
              "-c",
              "pg_dump --host=postgres --username=shell-history --format=c --file=/backup/shell-history-backup-$(date +'%Y-%m-%d').pg_dump",
            ]
            env:
              - name: PGPASSWORD
                valueFrom:
                  secretKeyRef:
                    name: shell-history-secrets
                    key: SUBNET_SHELL_HISTORY_DB_PASSWORD
                    optional: false
            volumeMounts:
            - name: backup-volume
              mountPath: /backup
          restartPolicy: OnFailure
          volumes:
          - name: backup-volume
            hostPath:
              path: /somewhere/on/node/for/database-backups
              type: Directory
```

Configure/update the example `yaml` with the following:
- Set a more or less frequent schedule with the `schedule` property.
- Replace `/somewhere/on/node/for/database-backups` with a path on your node or reconfigure to use a `PersistentVolume` instead of `hostPath`.
- `--format=c` outputs a format that can be restored with `pg_restore`. Use [`plain`](https://www.postgresql.org/docs/current/app-pgdump.html) if you want `.sql` files outputted instead.
