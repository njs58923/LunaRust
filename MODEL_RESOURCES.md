# Model resources and instances — stage 1

Each HSML `<model src="...">` owns a stable Bevy entity (`ModelInstance`). Its
transform, visibility, document identity and authored children survive source
changes. Only the generated `ModelContent` child is replaced.

`ModelResource` holds shared AssetServer handles for the scene and, for GLB/glTF,
the parent Gltf asset. Equal prepared paths share assets, while each model has
its own scene hierarchy. Dropping an instance releases its handles; other
instances retain theirs. Clip playback is implemented in stage 2 (see
[GLB_ANIMATIONS.md](GLB_ANIMATIONS.md)); a procedural mesh API is not implemented yet.

## Loading lifecycle

- `model-state="empty"`: no source (including whitespace).
- `model-state="loading"`: preparing the file or instantiating its scene.
- `model-state="ready"`: SceneSpawner has instantiated the current scene.
- `model-state="error"`: preparation or asset loading failed; `model-error`
  contains the error. Invalid source URLs also fail explicitly.

`model-source` records the source associated with the state. These are engine
output attributes published through the existing AttributeUpdates pipeline;
they are not commands or a new promise/event API. Ready describes scene
instantiation, not GPU upload completion.

IO `Prepared` means only that a local asset file exists. It must not be confused
with the public instance `ready` state. Requests are grouped by resolved URL and
checked against request IDs before their results can update nodes. Instance
generations additionally guard scene completion after visual replacement.
Relative sources resolve against the owning document, including included pages.

## Performance and cache

Transform-only updates keep the existing fast path. Only instances awaiting scene
completion enter the pending query; ready/error instances leave it. Downloads
waiting on IO do not poll SceneSpawner. An unchanged source and prepared path
does not rebuild the scene.

Prepared filenames hash both resolved URL and contents with BLAKE3. A later
revision cannot overwrite the file used by an earlier revision; concurrent
writers publish through unique temporary files and rename. Identical revisions
reuse their paths and Bevy assets. The existing disk-cache retention policy is
unchanged; this stage adds no eviction policy or external glTF dependency resolver.

Regression coverage lives in `models.rs` and `io.rs`: stable roots and authored
children, asset sharing with distinct instances, current-generation readiness,
cancelled/replaced requests, and immutable content revisions.
