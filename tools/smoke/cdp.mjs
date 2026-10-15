// A small Chrome DevTools Protocol client over Node's built-in WebSocket (Node 22+).
// No dependency: the browser is driven directly, and every call is one JSON message.

export class Cdp {
  #socket;
  #next = 1;
  #pending = new Map();
  #listeners = [];

  static async connect(url) {
    const cdp = new Cdp();
    cdp.#socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      cdp.#socket.addEventListener("open", resolve, { once: true });
      cdp.#socket.addEventListener("error", () => reject(new Error(`CDP socket failed: ${url}`)), { once: true });
    });
    cdp.#socket.addEventListener("message", (event) => cdp.#receive(JSON.parse(event.data)));
    return cdp;
  }

  #receive(message) {
    if (message.id !== undefined) {
      const waiting = this.#pending.get(message.id);
      this.#pending.delete(message.id);
      if (!waiting) return;
      if (message.error) waiting.reject(new Error(`${waiting.method}: ${message.error.message}`));
      else waiting.resolve(message.result ?? {});
      return;
    }
    for (const listener of this.#listeners) listener(message.method, message.params ?? {});
  }

  send(method, params = {}) {
    const id = this.#next++;
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject, method });
      this.#socket.send(JSON.stringify({ id, method, params }));
    });
  }

  on(listener) {
    this.#listeners.push(listener);
  }

  close() {
    this.#socket.close();
  }
}
