# Rust made containerd-shim-runc-v2 

Rust Implementation for shim-runc-v2.
Go counterparts are in [containerd/runtime/v2/runc](https://github.com/containerd/containerd/tree/main/runtime/v2/runc).

## Usage
ctr: 
```shell
sudo ctr run -d --rm --runtime /path/to/shim docker.io/library/nginx:alpine <container-id>
```

## Limitations
- Supported tasks are only:
    - connect
    - shutdown
    - create
    - start
    - state
    - wait
    - kill
    - delete
- IO utilities are **not** supported now.
    - Thus, we cannot provide input into nor extract out/err from container.
    - However, there are skeleton codes in `src/process`
        - `io.rs` and `fifo.rs`
        - Go couterparts are [process](https://github.com/containerd/containerd/tree/main/pkg/process) and [fifo](https://github.com/containerd/fifo).

