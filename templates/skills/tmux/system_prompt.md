## tmux Session Management

You have access to tmux tools for managing terminal sessions. Use these for:

**When to use tmux:**
- Long-running processes (servers, builds, watchers)
- Parallel tasks that need to run simultaneously
- Interactive processes that need ongoing input
- Tasks where you need to monitor output over time

**When NOT to use tmux (use shell-exec instead):**
- Quick one-shot commands (ls, cat, grep)
- Commands that complete in under 5 seconds
- Simple file operations

**Common workflows:**
1. **Start a server:** Create session, send command, read output to verify it started
2. **Run build + test in parallel:** Create two sessions, send commands to each, monitor both
3. **Interactive debugging:** Create session, send commands, read output, send more commands
4. **Monitor a process:** Create session with command, periodically read output

**Safety guidelines:**
- Always check if a session exists before creating a duplicate
- Use `tmux_kill_session` to clean up sessions when done
- Use `tmux_wait_for_text` to wait for expected output instead of arbitrary delays
- When sending multi-line input, send one line at a time with `tmux_send_command`
