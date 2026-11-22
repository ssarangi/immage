// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Auto-keywords: RAM++ inference on its own serial CPU worker (tags_analyze), and the
// keyword write the results go through (tags_apply). No editor session access.
// The app sends a preprocessed 3x384x384 float tensor file and gets the model's logits
// back; the tag list, the per-tag thresholds and the sigmoid live in the app
// (app/src-tauri/src/ram_models.rs), so this file knows nothing of the vocabulary.
#include "engine.h"
#include "common/tags.h"
#include <sys/stat.h>

#define TAGS_EDGE 384
#define TAGS_CLASSES 4585

static GMutex _tags_lock;
static GThreadPool *_tags_pool = NULL;
static int _tags_pending = 0;
static gboolean _tags_stopped = FALSE;
static FILE *_tags_out = NULL;

#ifdef HAVE_AI
static dt_ai_environment_t *_tags_env = NULL;
static dt_ai_context_t *_tags_model = NULL;
static char *_tags_root = NULL;

static void _tags_unload(void)
{
  if(_tags_model) { dt_ai_unload_model(_tags_model); _tags_model = NULL; }
  if(_tags_env) { dt_ai_env_destroy(_tags_env); _tags_env = NULL; }
  g_clear_pointer(&_tags_root, g_free);
}

// the model is about 1.8 GB on disk: it is loaded by the first analysis and freed
// by an `unload` request (the app sends one when the backlog is empty)
static dt_ai_context_t *_tags_load(const char *root, char **err)
{
  g_mutex_lock(&export_ai_load_lock);
  if(g_strcmp0(root, _tags_root)) _tags_unload();
  if(!_tags_env)
  {
    _tags_root = g_strdup(root);
    _tags_env = dt_ai_env_init(root);
    if(_tags_env) dt_ai_env_set_provider(_tags_env, DT_AI_PROVIDER_CPU);
  }
  if(_tags_env && !_tags_model)
    _tags_model = dt_ai_load_model(_tags_env, "immage-ram-plus", "../model.onnx", DT_AI_PROVIDER_CPU);
  dt_ai_context_t *ctx = _tags_model;
  g_mutex_unlock(&export_ai_load_lock);
  if(!ctx) *err = g_strdup("RAM++ model is unavailable; verify the Auto-keywords model");
  return ctx;
}

static gboolean _tags_do(JsonObject *req, JsonBuilder *reply, char **err)
{
  if(json_object_get_boolean_member_with_default(req, "unload", FALSE))
  {
    g_mutex_lock(&export_ai_load_lock);
    _tags_unload();
    g_mutex_unlock(&export_ai_load_lock);
    return TRUE;
  }
  const char *root = json_object_get_string_member_with_default(req, "model_root", "");
  const char *path = json_object_get_string_member_with_default(req, "input", "");
  if(!g_path_is_absolute(root) || !g_path_is_absolute(path))
  { *err = g_strdup("Invalid RAM++ tensor path"); return FALSE; }
  const size_t expected = 3 * TAGS_EDGE * TAGS_EDGE * sizeof(float);
  FILE *file = g_fopen(path, "rb");
  struct stat st;
  if(!file || fstat(fileno(file), &st) || !S_ISREG(st.st_mode) || st.st_size < 0 || (uint64_t)st.st_size != expected)
  { if(file) fclose(file); *err = g_strdup("Invalid RAM++ tensor file size or type"); return FALSE; }
  float *buffer = g_malloc(expected);
  const gboolean read_ok = fread(buffer, 1, expected, file) == expected && fgetc(file) == EOF && !ferror(file);
  fclose(file);
  if(!read_ok) { g_free(buffer); *err = g_strdup("Could not read RAM++ tensor"); return FALSE; }
  uint32_t *bits = (uint32_t *)buffer;
  for(size_t i = 0; i < expected / 4; i++)
  {
    bits[i] = GUINT32_FROM_LE(bits[i]);
    float value; memcpy(&value, &bits[i], 4);
    if(!isfinite(value) || fabsf(value) > 16.0f) { g_free(buffer); *err = g_strdup("Invalid RAM++ tensor values"); return FALSE; }
  }
  dt_ai_context_t *ctx = _tags_load(root, err);
  if(!ctx) { g_free(buffer); return FALSE; }
  int64_t in_shape[4] = { 1, 3, TAGS_EDGE, TAGS_EDGE };
  dt_ai_tensor_t input = { buffer, DT_AI_FLOAT, in_shape, 4 };
  float *logits = g_malloc0(TAGS_CLASSES * sizeof(float));
  int64_t out_shape[2] = { 1, TAGS_CLASSES };
  dt_ai_tensor_t output = { logits, DT_AI_FLOAT, out_shape, 2 };
  gboolean ok = dt_ai_get_input_count(ctx) == 1 && dt_ai_get_input_type(ctx, 0) == DT_AI_FLOAT
                && dt_ai_get_output_count(ctx) == 1 && dt_ai_get_output_type(ctx, 0) == DT_AI_FLOAT;
  if(!ok) *err = g_strdup("Unsupported RAM++ graph inputs, outputs or types");
  else if(dt_ai_run(ctx, &input, 1, &output, 1) < 0)
  { ok = FALSE; *err = g_strdup("CPU RAM++ inference failed"); }
  if(ok)
  {
    json_builder_set_member_name(reply, "logits"); json_builder_begin_array(reply);
    for(int i = 0; i < TAGS_CLASSES; i++)
    {
      if(!isfinite(logits[i])) { ok = FALSE; *err = g_strdup("RAM++ produced an invalid logit"); break; }
      json_builder_add_double_value(reply, logits[i]);
    }
    json_builder_end_array(reply);
  }
  g_free(logits);
  g_free(buffer);
  return ok;
}
#else
static gboolean _tags_do(JsonObject *req, JsonBuilder *reply, char **err)
{ (void)req; (void)reply; *err = g_strdup("This engine was built without CPU ONNX support"); return FALSE; }
#endif

static void _tags_reply_error(FILE *out, JsonObject *req, const char *error)
{
  JsonBuilder *b = json_builder_new(); json_builder_begin_object(b);
  json_builder_set_member_name(b, "tag"); json_builder_add_int_value(b, json_object_get_int_member_with_default(req, "tag", 0));
  json_builder_set_member_name(b, "ok"); json_builder_add_boolean_value(b, FALSE);
  json_builder_set_member_name(b, "error"); json_builder_add_string_value(b, error);
  json_builder_end_object(b); engine_reply(out, b); g_object_unref(b);
}

static void _tags_run(gpointer data, gpointer user)
{
  JsonParser *parser = data; JsonObject *req = json_node_get_object(json_parser_get_root(parser));
  JsonBuilder *b = json_builder_new(); char *error = NULL;
  json_builder_begin_object(b);
  json_builder_set_member_name(b, "tag"); json_builder_add_int_value(b, json_object_get_int_member_with_default(req, "tag", 0));
  const gint64 t0 = g_get_monotonic_time();
  const gboolean ok = _tags_do(req, b, &error);
  json_builder_set_member_name(b, "ok"); json_builder_add_boolean_value(b, ok);
  engine_add_double(b, "ms", engine_ms_since(t0));
  if(!ok) { json_builder_set_member_name(b, "error"); json_builder_add_string_value(b, error ? error : "RAM++ inference failed"); }
  json_builder_end_object(b); engine_reply(user, b);
  g_free(error); g_object_unref(b); g_object_unref(parser);
  g_mutex_lock(&_tags_lock); _tags_pending--; g_mutex_unlock(&_tags_lock);
}

void tags_start(FILE *out)
{
  _tags_out = out;
  _tags_stopped = FALSE;
}

// Takes ownership of parser on every path. At most four requests, including the active one.
void tags_queue(JsonParser *parser)
{
  JsonObject *req = json_node_get_object(json_parser_get_root(parser));
  g_mutex_lock(&_tags_lock);
  if(_tags_stopped || _tags_pending >= 4)
  { g_mutex_unlock(&_tags_lock); _tags_reply_error(_tags_out, req, "RAM++ worker is full or stopping"); g_object_unref(parser); return; }
  GError *error = NULL;
  if(!_tags_pool) _tags_pool = g_thread_pool_new(_tags_run, _tags_out, 1, TRUE, &error);
  if(!_tags_pool)
  {
    g_mutex_unlock(&_tags_lock);
    _tags_reply_error(_tags_out, req, error ? error->message : "Could not start the RAM++ worker");
    g_clear_error(&error); g_object_unref(parser); return;
  }
  _tags_pending++;
  const gboolean pushed = g_thread_pool_push(_tags_pool, parser, &error);
  if(!pushed) _tags_pending--;
  g_mutex_unlock(&_tags_lock);
  if(!pushed)
  { _tags_reply_error(_tags_out, req, error ? error->message : "RAM++ queue failed"); g_clear_error(&error); g_object_unref(parser); }
}

void tags_wait(void)
{
  g_mutex_lock(&_tags_lock); _tags_stopped = TRUE; GThreadPool *pool = _tags_pool; _tags_pool = NULL; g_mutex_unlock(&_tags_lock);
  if(pool) g_thread_pool_free(pool, FALSE, TRUE);
#ifdef HAVE_AI
  g_mutex_lock(&export_ai_load_lock); _tags_unload(); g_mutex_unlock(&export_ai_load_lock);
#endif
}

// Attaches and detaches darktable keywords the way keywords_set does, but says which
// ones it really changed: a keyword the user already had is never reported as added,
// so removing the AI tags later cannot take a keyword the user typed.
gboolean tags_apply(JsonObject *req, JsonBuilder *b, char **err)
{
  const dt_imgid_t imgid = json_object_get_int_member_with_default(req, "imgid", 0);
  if(!dt_is_valid_imgid(imgid)) { *err = g_strdup("no such image"); return FALSE; }
  JsonArray *add = json_object_has_member(req, "add") ? json_object_get_array_member(req, "add") : NULL;
  JsonArray *remove = json_object_has_member(req, "remove") ? json_object_get_array_member(req, "remove") : NULL;
  json_builder_set_member_name(b, "added");
  json_builder_begin_array(b);
  for(guint k = 0; add && k < json_array_get_length(add); k++)
  {
    const char *name = json_array_get_string_element(add, k);
    if(!name || !*name || g_str_has_prefix(name, "darktable|")) continue;
    guint tagid = 0;
    if(dt_tag_exists(name, &tagid) && dt_is_tag_attached(tagid, imgid)) continue;
    if(!dt_tag_new(name, &tagid)) continue;
    dt_tag_attach(tagid, imgid, FALSE, FALSE);
    json_builder_add_string_value(b, name);
  }
  json_builder_end_array(b);
  json_builder_set_member_name(b, "removed");
  json_builder_begin_array(b);
  for(guint k = 0; remove && k < json_array_get_length(remove); k++)
  {
    const char *name = json_array_get_string_element(remove, k);
    guint tagid = 0;
    if(!name || !dt_tag_exists(name, &tagid) || !dt_is_tag_attached(tagid, imgid)) continue;
    dt_tag_detach(tagid, imgid, FALSE, FALSE);
    json_builder_add_string_value(b, name);
  }
  json_builder_end_array(b);
  // TRUE means it failed
  if(library_write_sidecar(imgid))
  {
    *err = g_strdup("could not write the XMP sidecar");
    return FALSE;
  }
  return TRUE;
}
