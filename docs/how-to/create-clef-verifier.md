# Create a Clef verifier

Create a verifier when a domain needs a deterministic acceptance rule that is
not supplied by the built-in registry.

## Define the verifier

```python
from clef_sdk import CheckResult, CheckStatus


def content_marker(spec, context):
    output_name = spec.parameters["output"]
    artifact = context.outputs[output_name]
    return CheckResult(
        name=spec.name,
        status=CheckStatus.PASSED,
        message=f"validated {artifact.uri}",
        required=spec.required,
        evidence=(artifact,),
    )
```

A verifier receives a `VerifierSpec` and `VerificationContext`. It returns a
`CheckResult`. Raise an exception only when the verifier definition or runtime
environment is invalid; represent an unmet acceptance condition with a failed
`CheckResult`.

## Register the name

```python
from clef_sdk.verification import default_registry

registry = default_registry()
registry.register("content_marker", content_marker)
```

Names must be unique within a registry.

## Reference it from a contract

```python
from clef_sdk import VerifierSpec

spec = VerifierSpec(
    name="content_marker",
    parameters={"output": "result"},
)
```

Add `spec` to `DomainContract.verifiers`, then inject the registry:

```python
result = domain_run(
    task,
    profile=profile,
    verifier_registry=registry,
)
```

See [Verification and storage](../explanation/clef/verification-and-storage.md)
for execution order and
[Clef verification reference](../reference/clef/verification.md#verifierregistry)
for the
registry interface.
