# People command line

`immage faces ...` finds, names and reviews the faces in your photos from a terminal. It works on the same People data
as the app (the `faces.sqlite` in the app's data folder), so what you do here shows up in the app and the other way round,
even while the app is open. Everything stays on your computer: nothing is uploaded, nothing is written into your photo
folders, and the commands print counts, names and file paths, never face data or pictures.

People needs the **Faces** model, installed under Preferences > General > AI models. Faces is off in the app until you
turn it on there; the command line does not need it on, because you ask for each scan yourself.

```
immage faces status [--json]
immage faces scan <folder> [--no-subfolders] [--thorough] [--recognizer sface|arcface] [--allow-network] [--wait]
immage faces detect <image> [--thorough] [--json]
immage faces groups [--json]
immage faces people [--all] [--json]
immage faces person <name|id> [--images] [--json]
immage faces name <group-id> <name>
immage faces rename <person> <name>
immage faces merge <from> <into>
immage faces ignore <person> [--restore]
immage faces confirm <person> (<face-id>... | --suggested)
immage faces reject [--person <person>] <face-id>...
immage faces suggestions [--json]
immage faces export [--json]
immage faces strictness <loose|balanced|strict>
immage faces delete-all --yes
```

## Scanning

`faces scan <folder>` reads the photos in the folder (and its subfolders, unless `--no-subfolders`), finds the faces,
suggests who they are and groups the ones nobody is named for, with progress on standard error. It waits until the folder
is done (`--wait` is accepted and is the default). A scan can be interrupted and run again: photos already done are
skipped. The app's own scanning stands aside while a command line scan runs, so the two never read the same photos
together.

- `--thorough` looks at each photo at 1600 px and in tiles, which finds smaller faces and takes about four times as long.
- `--recognizer arcface` uses the ArcFace network when its local file is installed. A scan that uses other settings than the
  app does makes the app scan those photos again, with your names kept.
- Folders on a network share are refused unless you add `--allow-network`.

`faces detect <image>` finds the faces in one picture and prints their boxes, scores and pose, without touching the stored
data. Use it to see what the detector makes of a photo.

## Reviewing

Group numbers are made again after every change, so read `faces groups` again before the next `faces name`.

- `faces groups` lists the groups of unnamed faces; `faces name <group-id> <name>` names a whole group (a new person, or
  the person who already has that name).
- `faces people` lists the people with their photo counts (`--all` includes removed people); `faces person <name|id>` shows
  one, and `--images` prints their photos, one path per line.
- `faces rename`, `faces merge <from> <into>` and `faces ignore <person>` (`--restore` to bring them back) work on people,
  by name or id.
- `faces confirm <person> --suggested` accepts every face suggested for a person; with face numbers it names exactly those.
- `faces reject <face-id>...` marks "not a face"; with `--person <person>` it says the faces are not that person.
- `faces suggestions` lists groups that may be one person, groups that look like someone you named, and people who may be
  one, each with how sure Immage is. It applies nothing.
- `faces strictness` sets how readily faces are called the same person. While the app is open, set it in Preferences instead.
- `faces export --json` gives the people with their counts for other tools.
- `faces delete-all --yes` deletes every face, person and group, empties the face picture cache and switches Faces off.

## Output and exit codes

`--json` prints one JSON document to standard output on every command; progress and messages go to standard error.
`--runtime <folder>` points at another data folder, `--engine <path>` and `--ort <path>` name the engine and ONNX Runtime
when they are not found by themselves.

| Code | Meaning |
|---|---|
| 0 | done |
| 1 | the command failed |
| 2 | wrong usage, or a network folder without `--allow-network` |
| 3 | the face data could not be opened, or another scan is running |
| 4 | the Faces model or the engine is not available |
| 5 | done, but some photos could not be read |
