# SSH Agent
set -x SSH_AUTH_SOCK $XDG_RUNTIME_DIR/ssh-agent.socket

# KeePassXC as SSH askpass
set -x SSH_ASKPASS /usr/bin/ksshaskpass
set -x SSH_ASKPASS_REQUIRE force

# Loop until ssh keys are loaded in agent
function ssh --wraps /usr/bin/ssh
    set -l attempts 0
    while not /usr/bin/ssh-add -l >/dev/null 2>&1
        set attempts (math $attempts + 1)
        if test $attempts -gt 3
            echo "Failed to load SSH keys after 3 attempts" >/dev/stderr
            return 1
        end
        # attempt to unlock the database as it integrates
        /usr/bin/secret-tool lookup false false
    end
    /usr/bin/ssh $argv
end
