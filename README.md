# mqtt2tcp

An attempt to publish mqtt message payload to one or more TCP connected clients.

The reason I'm attempting to build this is because I need something similar to
[ser2net](http://ser2net.sourceforge.net). The power meter in my home publishes
usage statistics every 10 seconds and I'd like to import those in the 
[Home Assistant DSMR integration](https://www.home-assistant.io/integrations/dsmr/).

This is also just an experiment, so don't expect anything from it (yet).

Objectives:
* Listen on a (configurable) TCP port for clients
* Subscribe to an MQTT topic and receive message
* Write receive message payloads to connected TCP clients

Fair warning: I'm not very proficient with Rust and I'm still learning and
struggling ;)
