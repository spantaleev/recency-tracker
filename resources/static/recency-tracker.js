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


	var STATE_DISCONNECTED = 0;
	var STATE_CONNECTING = 1;
	var STATE_CONNECTED = 2;


	window.RecencyTracker = function (serverUrl) {
		this._serverUrl = serverUrl;
		this._sock = null;
		this._sockMessagesQueue = [];
		this._subscriptions = {};
		this._subscriptionsCount = 0;
		this._state = STATE_DISCONNECTED;
	};

	RecencyTracker.prototype.subscribe = function (resource, version, onChange) {
		if (resource in this._subscriptions) {
			var subscription = this._subscriptions[resource];
			subscription.addChangeHandler(onChange);
		} else {
			var subscription = new Subscription(resource, version, [onChange]);
			this._subscriptions[resource] = subscription;
			this._subscriptionsCount += 1;

			//Announce immediately only if connected.
			//Otherwise wait for the initial connection, which will announce all subscriptions.
			if (this._state === STATE_CONNECTED) {
				this._announce(subscription);
			} else {
				this._sockInit();
			}
		}
	};

	RecencyTracker.prototype.unsubscribe = function (resource) {
		if (! (resource in this._subscriptions)) {
			return;
		}

		var subscription = this._subscriptions[resource];

		this._sockSend({
			"type": "unsubscribe",
			"resource": resource
		});

		delete this._subscriptions[resource];
		this._subscriptionsCount -= 1;

		if (this._subscriptionsCount === 0) {
			//Don't keep an open connection for no subscriptions.
			this._disconnect();
		}
	};

	RecencyTracker.prototype._sockInit = function () {
		if (this._state !== STATE_DISCONNECTED) {
			return;
		}

		this._state = STATE_CONNECTING;

		var self = this;

		var init = function () {
			self._sock = new SockJS(self._serverUrl);

			self._sock.onopen = function() {
				self._state = STATE_CONNECTED;

				for (var resource in self._subscriptions) {
					self._announce(self._subscriptions[resource]);
				}

				for (var idx in self._sockMessagesQueue) {
					self._sock.send(self._sockMessagesQueue[idx]);
				}
				self._sockMessagesQueue = [];
			};

			self._sock.onclose = function() {
				self._state = STATE_CONNECTING;

				window.setTimeout(function () {
					init();
				}, 5000);
			};

			self._sock.onmessage = function (e) {
				var message = JSON.parse(e.data);
				self._onMessage(message);
			};
		};

		init();
	};

	RecencyTracker.prototype._sockSend = function (message) {
		var message = JSON.stringify(message);

		if (this._state === STATE_CONNECTED) {
			this._sock.send(message);
		} else {
			this._sockMessagesQueue.push(message);
			this._sockInit();
		}
	};

	RecencyTracker.prototype._onMessage = function (message) {
		if (message.type === "update") {
			var resource = message.resource,
				version = message.version,
				data = message.data;

			if (resource in this._subscriptions) {
				this._subscriptions[resource].update(version, data);
			}
		} else {
			throw new Error("Unknown message type: " + JSON.stringify(message));
		}
	};

	RecencyTracker.prototype._announce = function (subscription) {
		this._sockSend({
			"type": "subscribe",
			"resource": subscription.resource,
			"version": subscription.version
		});
	};

	RecencyTracker.prototype._disconnect = function () {
		if (this._sock !== null) {
			this._sock.onclose = function () {};
			this._sock.close();
			this._state = STATE_DISCONNECTED;
		}
	};
})();
