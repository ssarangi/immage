# Statistics

Statistics turns the metadata of your library into a picture of how you shoot:
which cameras and lenses you use, which focal lengths you keep, and how many
of your frames you rate as keepers. Open it from the chart button on the rail,
or View > Statistics. Esc or **Done** closes it, and the editor is as you left
it. Everything is read from Immage's local search index, so nothing leaves your
computer.

## Reading the page

- **Overview**: a headline, a coverage line and six cards: photos, top images,
  gear, favourite focal length, median setup and peak hour. Click a card to
  clear the filters (Photos), keep only top images (Top images), jump to
  Cameras (Gear), filter to your favourite focal range, or filter to your peak
  hour and month.
- **Cameras**: the year chips show how many photos you shot in each year
  (**That year**) or up to it (**Cumulative**); the maker donut and its legend
  show each maker's share; Keeper rate by body ranks bodies by the share of
  their frames that reached your top-images rating; Top bodies shows the best
  frame of each body.
- **Lenses**: a ranking by most used, keeper rate or top images, either per
  exact lens model or grouped by focal range, and a field-of-view fan: each
  wedge is a focal range at its real angle of view on a 35 mm frame, and the
  busier the range, the longer its wedge. Hatched bars come from fewer than 100
  photos and are a hint rather than a measure.

Exposure, Time, Places and Top images sections are added in later releases.

## Filtering by clicking

Every bar, slice, row, chip and wedge is a filter. Click one to set it, click
it again to clear it, click another in the same chart to replace it, and
Shift- or Cmd/Ctrl-click to add to it. The whole page re-aggregates, except
that each chart ignores its own filter so you can still see the alternatives.
Photo cards never filter: they open the photo. (On Top bodies the card filters
and a small button on the picture opens the frame.)

Active filters are chips in the bar under the tabs; the x removes one, **Clear
all** removes all, and Esc removes the last one added (a second Esc closes the
view). The bar always shows how many photos match and how many of them are top
images. **Open N in grid** shows the matching photos in the grid, best rated
first, as search results are, and opens them like any other photo (edits go to
the photo's own sidecar). It shows at most 20,000 photos; with more matches,
narrow the filter. The Grid button on a maker row opens just that maker's
photos.

## Scope, top images and labels

- **Scope**: Whole library, This folder (the folder open in the Folders panel)
  or a Date range, with presets for this year, last year and the last 12
  months. Filters that cannot exist in the new scope are dropped.
- **Top images**: the stepper sets the rating that counts as "top" (1 to 5
  stars, default 4). It defines the amber parts of the bars, the top counts and
  the keeper rate everywhere, and is remembered. It does not filter by itself.
- **Label**: the five colour dots filter by colour label; select several to
  match any of them.

## Coverage

The line under the headline says what the numbers are based on: how many photos
are analysed, how many are still being indexed, and **Add folders...** (the
same as the Library panel's). While indexing runs the counts update every two
seconds, and the charts refresh once when it finishes; **Pause** and **Resume**
control the indexing. A section's pill (for example "44,602 / 46,880 photos with
camera data") shows how many photos had the data that chart needs. With no
folders added, the page explains what is needed instead of showing empty charts.
Refresh reads the numbers again; they also update about a second after a rating
changes.

Keyboard: every control is focusable; arrow keys move between the marks of a
chart and Enter or Space press one.
