var RecencyTracker = function (serverUrl) {
	this.serverUrl = serverUrl;
	this.sock = null;
	this.sockMessagesQueue = [];
	this.sockReady = false;
	this.isExplicitClose = false;
	this.initMessages = [];
	this.resourceCallbacks = [];
};

RecencyTracker.prototype.sockInit = function () {
	if (this.sock !== null) {
		return;
	}

	this.sock = new SockJS(this.serverUrl);

	var self = this;

	this.sock.onopen = function() {
		for (var idx in self.sockMessagesQueue) {
			var message = self.sockMessagesQueue[idx];
			self.sock.send(message);
		}

		self.sockMessagesQueue = [];
		self.sockReady = true;
	};

	this.sock.onclose = function() {
		if (self.isExplicitClose) {
			//Ignore just this one close.
			self.isExplicitClose = false;
			return;
		}

		//While reconnecting, this `onclose()` callback will be called
		//once for each failed attempt.

		var wasReady = self.sockReady;

		self.sock = null;
		self.sockReady = false;

		if (wasReady) {
			//This is a disconnect after a succesfull connection (the real disconnect).
			//Schedule all init messages for resending, before we issue a reconnect.
			for (var idx in self.initMessages) {
			   self.sockMessagesQueue.push(self.initMessages[idx]);
			}
		}

		//Try to reconnect soon.
		window.setTimeout(function () {
			self.sockInit();
		}, 1000);
	};

	this.sock.onmessage = function (e) {
		var message = JSON.parse(e.data);
		self.onMessage(message);
	};
};

RecencyTracker.prototype.sockSend = function (message, isInitMessage) {
	var message = JSON.stringify(message);

	if (isInitMessage) {
		//Keep track of this initialization message, so we can rebuild
		//the connection if it fails.
		this.initMessages.push(message);
	}

	if (this.sockReady) {
		this.sock.send(message);
	} else {
		this.sockMessagesQueue.push(message);
	}

	this.sockInit();
};

RecencyTracker.prototype.onMessage = function (message) {
	if (message.type === "update") {
		this.onMessageUpdate(message);
	} else {
		alert("Unknown message type: " + JSON.stringify(message));
	}
};

RecencyTracker.prototype.onMessageUpdate = function (message) {
	var resource = message.resource,
		version = message.version,
		data = message.data;

	if (typeof(this.resourceCallbacks[resource]) === 'undefined') {
		return;
	}

	var callbacks = this.resourceCallbacks[resource];
	for (var idx in callbacks) {
		var callback = callbacks[idx];
		callback(version, data);
	}
};

RecencyTracker.prototype.subscribe = function (resource, version, onChange) {
	if (typeof(this.resourceCallbacks[resource]) === 'undefined') {
		//Initial subscription announce to the server.
		this.resourceCallbacks[resource] = [onChange];

		this.sockSend({
			"type": "subscribe",
			"resource": resource,
			"version": version
		}, true);
	} else {
		//Another subscription for the same resource - do not announce.
		this.resourceCallbacks[resource].push(onChange);
	}
};

RecencyTracker.prototype.unsubscribe = function (resource) {
	delete this.resourceCallbacks[resource];

	this.sockSend({
		"type": "unsubscribe",
		"resource": resource
	});

	if (this.resourceCallbacks.length === 0) {
		//Last resource closed. Let's close the connection as well.
		this.destroy();
	}
};

RecencyTracker.prototype.destroy = function () {
	if (this.sock === null) {
		return;
	}

	this.isExplicitClose = true;
	this.sock.close();
};
