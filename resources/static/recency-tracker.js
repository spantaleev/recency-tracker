var RecencyTracker = function (serverUrl) {
	this.serverUrl = serverUrl;
	this.sock = null;
	this.sockMessagesQueue = [];
	this.sockReady = false;
	this.isExplicitClose = false;
	this.initMessages = [];
	this.channelCallbacks = [];
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
	var channel = message.channel,
		version = message.version;

	if (typeof(this.channelCallbacks[channel]) === 'undefined') {
		return;
	}

	var callbacks = this.channelCallbacks[channel];
	for (var idx in callbacks) {
		var callback = callbacks[idx];
		callback(version);
	}
};

RecencyTracker.prototype.subscribe = function (channel, version, onChange) {
	if (typeof(this.channelCallbacks[channel]) === 'undefined') {
		this.channelCallbacks[channel] = [];
	}

	this.channelCallbacks[channel].push(onChange);

	this.sockSend({
		"type": "subscribe",
		"channel": channel,
		"version": version
	}, true);
};

RecencyTracker.prototype.unsubscribe = function (channel) {
	delete this.channelCallbacks[channel];

	this.sockSend({
		"type": "unsubscribe",
		"channel": channel
	});

	if (this.channelCallbacks.length === 0) {
		//Last channel closed. Let's close the connection as well.
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
