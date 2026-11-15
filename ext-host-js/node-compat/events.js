// Node.js events.EventEmitter shim

class EventEmitter {
  constructor() {
    this._events = new Map();
  }

  on(event, listener) {
    if (!this._events.has(event)) {
      this._events.set(event, []);
    }
    this._events.get(event).push(listener);
    return this;
  }

  addListener(event, listener) {
    return this.on(event, listener);
  }

  once(event, listener) {
    const wrapper = (...args) => {
      this.removeListener(event, wrapper);
      listener.apply(this, args);
    };
    return this.on(event, wrapper);
  }

  emit(event, ...args) {
    const listeners = this._events.get(event);
    if (!listeners || listeners.length === 0) return false;
    for (const fn of listeners.slice()) {
      fn.apply(this, args);
    }
    return true;
  }

  removeListener(event, listener) {
    const listeners = this._events.get(event);
    if (!listeners) return this;
    const idx = listeners.indexOf(listener);
    if (idx !== -1) listeners.splice(idx, 1);
    return this;
  }

  off(event, listener) {
    return this.removeListener(event, listener);
  }

  removeAllListeners(event) {
    if (event) {
      this._events.delete(event);
    } else {
      this._events.clear();
    }
    return this;
  }
}

module.exports = EventEmitter;
module.exports.EventEmitter = EventEmitter;
