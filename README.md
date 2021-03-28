# elayday

elayday is a distributed key-value store inspired by [delay-line memory](https://en.wikipedia.org/wiki/Delay_line_memory). It uses UDP to continuously forward data between two or more nodes, effectively storing data in the network between them.

## Setup

The easiest way to try out elayday is via Docker:

```
docker run -it ghcr.io/tibordp/elayday:latest server --bind [::1]:24601 --destination [::1]:24602
```

(If you get `Error: IOError(Os { code: 99, kind: AddrNotAvailable, message: "Address not available" })`, you should look [into enabling IPv6 for Docker](https://docs.docker.com/config/daemon/ipv6/). If you do not care about IPv6, you really should, but anyway, you can also bind on `127.0.0.1:<port>`)

Elayday is now running, but is sending packets into the void, so in order for the delay line to be closed, something needs to be sending the packets back.

You may be tempted to try

```
docker run -it ghcr.io/tibordp/elayday:latest server \
    --bind [::1]:24602 \
    --destination [::1]:24601
```

This would indeed work, but since you are still on localhost, there is not enough delay, so the UDP packets will be looping back so quickly to drain all the CPU. Ideally, you would run the other node somewhere on the other side of the internet. In fact, the destination server does not even have to run elayday, any UDP refelector would work just fine.

If you want to cheat or just test it out, you may run a reflector which will add artificial delay locally:

```
docker run -it ghcr.io/tibordp/elayday:latest reflector \
    --bind [::1]:24602 \
    --delay <number of seconds>
```

Setup like this is of course not a real delay-line, as most of the time, the messages are stored in memory on the reflector.

### Multi-node setup

An elayday cluster can run on as many nodes as you want, as long as they are arranged in ring topology (destination of each node is set to the address of the next one in the ring and the ring is closed).

## Interacting with the API

elayday has a gRPC interface, which allows you to store and retreive data from the delay line. In order to enable gRPC, add `--bind-grpc <address>:<port>` command line option, for example:

```
docker run -it -p 24603:24603 \
    ghcr.io/tibordp/elayday:latest server \
    --bind [::1]:24601 \
    --destination [::1]:24602 \
    --bind-grpc [::1]:24603
```

For example, using [gRPCurl](https://github.com/fullstorydev/grpcurl).


### Store a key
```
grpcurl -plaintext \
  -d '{"key":"foo","value":"SGVsbG8gd29ybGQgCg=="}' \
  [::1]:24603 elayday.Elayday/PutValue
```

### Retreive the key
```
grpcurl -plaintext \
    -d '{"key":"foo"}' \
    [::1]:24603 elayday.Elayday/GetValue
{
  "value": "SGVsbG8gd29ybGQgCg=="
}
```

## Limitations

There are no limitations on the size of the values stored (well, it has to fit into memory of the client and server) as elayday will automatically split the large message into UDP packets not exceeding common MTU over the public internet.

Capacity of the delay-line is directly proportional to the product of network bandwidth and the duration of the round-trip. For example, a 10 Gbps of bandwidth and a 200 ms delay will give a capacity of around 250 megabytes.

For best results wraps 1000 turns of optical fibre around the Earth's equator or put a mirror on Pluto.

## Development

Build from source with nighlty Rust:

```
cargo +nightly build
``` 

A full-featured [Tilt](https://tilt.dev/) environment is available running elayday in your local Kubernetes cluster.

```
tilt up
```

The gRPC API will be available on `localhost:8080`

## Roadmap

- Ability to delete keys
- Ability to detect non-existent keys on GET operation. Currently it will block until the key appears.
- Forward error correction to deal with lost packets 

## Is this actually useful?

No.