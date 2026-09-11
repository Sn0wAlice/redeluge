# DelugeRPC

The wire protocol `redeluged` speaks on port 58846. It is unchanged from the
Python daemon, because every existing client speaks it. The framing and its two
limits live in `crates/redeluge-rpc/src/transfer.rs`.

## Framing

Every message travels inside a frame over a TLS connection:

```
byte 0      protocol version, always 1
bytes 1-4   body length, big-endian u32
bytes 5..   the body: a zlib stream of one rencode-encoded value
```

Two limits guard the reader, because a compressed body declares its size only
after it has been decompressed: a frame is refused above 16 MiB, and a body
above 64 MiB. Without the second one a 200 KB frame expands to 200 MB.

## Message formats

There are four message types. A request carries no type id of its own; the
three the daemon sends are numbered.

| Id | Message |
|---|---|
| 1 | RPC response |
| 2 | RPC error |
| 3 | Event |

### RPC request

Sent by the client to call a remote method. Several requests can be bundled in
one list.

```
[[request_id, method, [args], {kwargs}], ...]
```

- **request_id** (int) chosen by the client, echoed in the reply. Replies are
  not ordered, so this is how the client pairs them up. Ordering the replies
  instead would stall every later call behind a slow one.
- **method** (str) the method to call, dotted to reach another object.
- **args** (list) positional arguments.
- **kwargs** (dict) keyword arguments.

### RPC response

The return value of a call. An error replaces it rather than accompanying it.

```
[1, request_id, [return_value]]
```

### RPC error

```
[2, request_id, exception_type, exception_msg, traceback]
```

- **exception_type** (str) the class name of the exception raised.
- **exception_msg** (str) why it was raised.
- **traceback** (str) the traceback.

### Event

Sent by the daemon on its own, for state the clients need to follow.

```
[3, event_name, data]
```

- **event_name** (str) the event being emitted.
- **data** (list) whatever that event carries.

The 22 events are listed in `contract/events.json` in the repository.
