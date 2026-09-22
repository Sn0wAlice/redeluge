# OpenAPI

The HTTP API has a machine-readable specification:
[`docs/openapi.yaml`](https://github.com/retorrent/redeluge/blob/main/docs/openapi.yaml)
in the repository. OpenAPI 3.1, one schema per method, 100 of them.

It describes the API of the Python Deluge as much as of redeluge, because they
are the same API. That is the point of the fork: a client generated from this
file works against either.

## What is in it

Two paths carry everything: `POST /json`, whose request body is a union over
every method discriminated by the `method` field, and `POST /upload` for the
add-by-file dialog. Each method's schema pins the
method name, lists its parameters in order with which are required, and carries
its authorisation level and the file in the Python implementation that defined
it.

Three other paths are described for completeness: the page itself, the rendered
template fragments the front end fetches, and the static assets.

## Generated, not written

The specification is produced from the frozen contract by
`tools/gen_openapi.py`, which reads `contract/rpc-api.json`. The contract in
turn was extracted from the Python source by parsing it, not by transcribing
it. Nothing in this chain is typed by hand.

```bash
python3 tools/gen_openapi.py            # write it
python3 tools/gen_openapi.py --check    # fail if it is out of date
```

The `--check` form runs in the test gate, so a specification that has drifted
from the contract fails the build. A hand-written API document drifts silently,
and a drifted document is worse than none.

## Using it

Read it in any OpenAPI viewer, or generate a client:

```bash
npx @redocly/cli preview-docs docs/openapi.yaml
```

## What `result` holds

Forty-five of the ninety-nine methods carry a schema for what they put in
`result`, named `<Method>Result` in the components. They come from the type
annotations the Python source declared, translated by the generator, so they
cannot drift either. The other fifty-four either declared nothing or return a
class whose fields the contract does not record; the specification says which
rather than guessing.

Those schemas are not referenced from the response, and cannot be: one endpoint
carries every method, and OpenAPI selects a response by status code, not by
request body. A generated client has to pick the right one by the method it
called. For what a call actually gives back in practice, the worked examples on
[Web API](Web-API) are still more use.

## One method the contract could not see

`system.listMethods` is answered by the JSON handler itself rather than being
declared with the decorator the extractor looks for, so it is absent from
`contract/rpc-api.json` and present on the wire. The generator declares it
explicitly, with that noted in its description, so the specification matches
what the server answers rather than what the extractor could find.

## The daemon's own protocol

This specification covers the HTTP API. The daemon speaks a binary protocol on
port 58846 that OpenAPI cannot describe; it is written out on
[DelugeRPC](DelugeRPC), and the same 70 `core.*` and `daemon.*` methods are
reachable over it.
