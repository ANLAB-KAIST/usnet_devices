# usnet_devices: Ethernet interface types for smoltcp, usnet_sockets, and usnetd
See [usnet_sockets](https://github.com/ANLAB-KAIST/usnet_sockets) and [usnetd](https://github.com/ANLAB-KAIST/usnetd) for a description.

This here is kept as separate repository and not moved to smoltcp nor to usnetd or usnet_sockets for the following reasons.
The TAP device type is modified from smoltcp, and if the correct MTU would be used in smoltcp, just the macvtap code could be moved in smoltcp if accepted there.
But anyway smoltcp does not need to have netmap support and even zero-copy forwarding in its repository and it makes more sense to maintain this separately.

# Features
The `netmap` feature is optional and requires the netmap and netmap_user C headers to be available for compilation.
At runtime the netmap kernel module must be loaded if netmap is to be used.

