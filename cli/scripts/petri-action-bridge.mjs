import {spawn} from 'node:child_process';

// One resident Rust process and one in-flight request. Failed executions are
// never replayed, including on process loss, invalid output or a timeout.
export class ResidentPetri {
  constructor({command, environment, cwd, timeoutMs = 300_000}) {
    this.options = {command, environment, cwd, timeoutMs};
    this.queue = Promise.resolve();
    this.sequence = 0;
    this.child = null;
    this.active = null;
  }
  invoke(action, args) {
    const request = {id: ++this.sequence, action, arguments: args};
    const wire = JSON.stringify(request) + '\n';
    if (Buffer.byteLength(wire) > 26 * 1024) throw new Error('Public action request exceeds its size bound');
    const result = this.queue.then(() => this.send(request, wire));
    this.queue = result.catch(() => {});
    return result;
  }
  send(request, wire) {
    if (!this.child) this.start();
    const child = this.child;
    return new Promise(resolve => {
      const timer = setTimeout(() => this.fail(child, 'Agent action timed out.'), this.options.timeoutMs);
      this.active = {request, resolve, timer};
      child.stdin.write(wire, error => {
        if (error) this.fail(child, 'Agent request transport closed.');
      });
    });
  }
  start() {
    const command = this.options.command();
    const child = spawn(command.bin, [...command.prefix, '--json', 'mcp', 'serve'], {
      cwd: this.options.cwd, env: this.options.environment(), windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.child = child;
    let buffer = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      if (this.child !== child) return;
      buffer += chunk;
      if (Buffer.byteLength(buffer) > 8 * 1024 * 1024) return this.fail(child, 'Agent response exceeds its bound.');
      let boundary;
      while ((boundary = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, boundary); buffer = buffer.slice(boundary + 1);
        let response;
        try { response = JSON.parse(line); } catch { return this.fail(child, 'Invalid agent response.'); }
        if (!this.active || response.id !== this.active.request.id || !response.result || typeof response.result !== 'object') {
          return this.fail(child, 'Agent response identity changed.');
        }
        const active = this.active; this.active = null;
        clearTimeout(active.timer);
        active.resolve(response.result);
      }
    });
    // Diagnostics may contain private local paths; never forward them to tools.
    child.stderr.on('data', () => {});
    child.stdin.on('error', () => this.fail(child, 'Agent request transport closed.'));
    child.on('error', () => this.fail(child, 'Resident Petri unavailable.'));
    child.on('exit', () => this.fail(child, 'Resident Petri exited.'));
  }
  fail(child, message) {
    if (this.child !== child) return;
    this.child = null;
    child?.kill();
    const active = this.active; this.active = null;
    if (!active) return;
    clearTimeout(active.timer);
    const execution = active.request.action === 'operations.execute';
    active.resolve({ok:false, retryAuthorized:false, operationId:active.request.arguments.operationId,
      error:{code:execution ? 'OPERATION_TRANSPORT_UNCERTAIN' : 'AGENT_TRANSPORT_UNAVAILABLE',
        category:execution ? 'pending' : 'unavailable', message, retryable:false},
      nextStep:execution ? 'Recover the original operation; never automatically resubmit.' : 'Reconnect the agent interface.'});
  }
  async close() {
    await this.queue;
    this.child?.stdin.end();
  }
}
