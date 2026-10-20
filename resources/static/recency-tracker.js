/** recency-tracker 2.0 - BSD licensed - https://github.com/spantaleev/recency-tracker **/

(function () {
	var Subscription = function (resource, version, changeHandlers) {
		this.resource = resource;
		this.version = version;
		this.changeHandlers = changeHandlers;
	};

	Subscription.prototype.addChangeHandler = function (callback) {
		this.changeHandlers.push(callback);
	};

	Subscription.prototype.update = function (version, data) {
		this.version = version;
		for (var idx in this.changeHandlers) {
			this.changeHandlers[idx](version, data);
		}
	};

	window.RecencyTracker = function (serverUrl) {
		var url = new URL(serverUrl, window.location.href);
		if (url.protocol === 'http:') {
			url.protocol = 'ws:';
		} else if (url.protocol === 'https:') {
			url.protocol = 'wss:';
		}
		if ((url.protocol !== 'ws:' && url.protocol !== 'wss:') || url.hash) {
			throw new Error('Expected a WebSocket or HTTP URL without a fragment');
		}

		this._serverUrl = url.href;
		this._sock = null;
		this._reconnectTimer = null;
		this._subscriptions = Object.create(null);
		this._subscriptionsCount = 0;
	};

	RecencyTracker.prototype.subscribe = function (resource, version, onChange) {
		version = ('' + version);
		if (resource in this._subscriptions) {
			this._subscriptions[resource].addChangeHandler(onChange);
			return;
		}

		var subscription = new Subscription(resource, version, [onChange]);
		this._subscriptions[resource] = subscription;
		this._subscriptionsCount += 1;

		if (this._sock !== null && this._sock.readyState === WebSocket.OPEN) {
			this._announce(subscription);
		} else {
			this._sockInit();
		}
	};

	RecencyTracker.prototype.unsubscribe = function (resource) {
		if (!(resource in this._subscriptions)) {
			return;
		}

		delete this._subscriptions[resource];
		this._subscriptionsCount -= 1;
		if (this._subscriptionsCount === 0) {
			this._disconnect();
		} else if (this._sock !== null && this._sock.readyState === WebSocket.OPEN) {
			this._sock.send(JSON.stringify({ type: 'unsubscribe', resource: resource }));
		}
	};

	RecencyTracker.prototype._sockInit = function () {
		if (this._subscriptionsCount === 0 || this._sock !== null || this._reconnectTimer !== null) {
			return;
		}

		var self = this;
		var sock = new WebSocket(this._serverUrl);
		this._sock = sock;

		sock.onopen = function () {
			if (self._sock !== sock) {
				return;
			}
			for (var resource in self._subscriptions) {
				self._announce(self._subscriptions[resource]);
			}
		};

		sock.onclose = function () {
			if (self._sock !== sock) {
				return;
			}
			self._sock = null;
			if (self._subscriptionsCount > 0) {
				self._reconnectTimer = window.setTimeout(function () {
					self._reconnectTimer = null;
					self._sockInit();
				}, 5000);
			}
		};

		sock.onmessage = function (event) {
			if (self._sock !== sock) {
				return;
			}
			var message;
			try {
				message = JSON.parse(event.data);
			} catch (error) {
				return;
			}
			self._onMessage(message);
		};
	};

	RecencyTracker.prototype._onMessage = function (message) {
		if (message !== null && typeof message === 'object' && message.type === 'update' &&
			typeof message.version === 'string' && typeof message.resource === 'string' &&
			message.resource in this._subscriptions) {
			this._subscriptions[message.resource].update(message.version, message.data);
		}
	};

	RecencyTracker.prototype._announce = function (subscription) {
		this._sock.send(JSON.stringify({
			type: 'subscribe',
			resource: subscription.resource,
			version: subscription.version
		}));
	};

	RecencyTracker.prototype._disconnect = function () {
		if (this._reconnectTimer !== null) {
			window.clearTimeout(this._reconnectTimer);
			this._reconnectTimer = null;
		}
		var sock = this._sock;
		this._sock = null;
		if (sock !== null) {
			sock.onopen = sock.onmessage = sock.onclose = null;
			sock.close();
		}
	};
})();
