# Rootless podman is the docker API for CI and test suites. The socket belongs
# to martin, so a container escape lands as martin, not root. The rootful
# daemon stays reachable through `sudo docker` and `sudo ci-docker`, because
# sudo drops DOCKER_HOST.
# Set it only when the socket exists, so docker still works before setup.
if test -S $XDG_RUNTIME_DIR/podman/podman.sock
    set -x DOCKER_HOST unix://$XDG_RUNTIME_DIR/podman/podman.sock
end
