// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Face models (YuNet, SFace, OCEC) on their own serial CPU worker, through the same
// darktable AI layer as clip.c and tags.c. The command is generic: the app writes one float
// tensor to a file, names a model of an installed pack, and gets every output tensor back in
// a file, with its shape. Pre and post processing (letterbox, decode, NMS, alignment) live in
// the app (app/src-tauri/src/faces_detect.rs), where they are unit tested; no photo and no
// session state is touched here.
#include "engine.h"
#include <sys/stat.h>

#define FACES_MAX_LOADED 4
#define FACES_MAX_OUT 16
#define FACES_MAX_BYTES (64u << 20)

static GMutex _faces_lock;
static GThreadPool *_faces_pool = NULL;
static int _faces_pending = 0;
static gboolean _faces_stopped = FALSE;
static FILE *_faces_out = NULL;

#ifdef HAVE_AI
typedef struct
{
  char *key;
  dt_ai_environment_t *env;
  dt_ai_context_t *ctx;
} _faces_model_t;
static _faces_model_t _faces_models[FACES_MAX_LOADED];

static void _faces_unload_all(void)
{
  for(int i = 0; i < FACES_MAX_LOADED; i++)
  {
    if(_faces_models[i].ctx) dt_ai_unload_model(_faces_models[i].ctx);
    if(_faces_models[i].env) dt_ai_env_destroy(_faces_models[i].env);
    g_free(_faces_models[i].key);
    _faces_models[i] = (_faces_model_t){ 0 };
  }
}

// a name made of letters, digits and "-_." only
static gboolean _faces_plain(const char *s)
{
  if(!s || !*s || strlen(s) > 80 || s[0] == '.') return FALSE;
  for(const char *p = s; *p; p++)
    if(!g_ascii_isalnum(*p) && *p != '-' && *p != '_' && *p != '.') return FALSE;
  return TRUE;
}

// "name.onnx" or "folder/name.onnx", every part plain
static gboolean _faces_model_path(const char *s)
{
  if(!s || !g_str_has_suffix(s, ".onnx")) return FALSE;
  gchar **parts = g_strsplit(s, "/", 0);
  const guint n = g_strv_length(parts);
  gboolean ok = n >= 1 && n <= 2;
  for(guint i = 0; ok && i < n; i++) ok = _faces_plain(parts[i]);
  g_strfreev(parts);
  return ok;
}

// root/engine/<id>/config.json declares the model id; the file is root/models/<file>
static dt_ai_context_t *_faces_load(const char *root, const char *id, const char *file, char **err)
{
  char *key = g_strdup_printf("%s|%s|%s", root, id, file);
  g_mutex_lock(&export_ai_load_lock);
  dt_ai_context_t *found = NULL;
  for(int i = 0; i < FACES_MAX_LOADED && !found; i++)
    if(_faces_models[i].key && !strcmp(_faces_models[i].key, key)) found = _faces_models[i].ctx;
  if(!found)
  {
    int slot = -1;
    for(int i = 0; i < FACES_MAX_LOADED && slot < 0; i++)
      if(!_faces_models[i].ctx) slot = i;
    if(slot < 0) { _faces_unload_all(); slot = 0; }
    char *search = g_build_filename(root, "engine", NULL);
    dt_ai_environment_t *env = dt_ai_env_init(search);
    g_free(search);
    if(env)
    {
      dt_ai_env_set_provider(env, DT_AI_PROVIDER_CPU);
      char *rel = g_strdup_printf("../../models/%s", file);
      // the model file is looked up inside the model's own folder, so id and file are checked above
      found = dt_ai_load_model(env, id, rel, DT_AI_PROVIDER_CPU);
      g_free(rel);
      if(found) _faces_models[slot] = (_faces_model_t){ key, env, found };
      else dt_ai_env_destroy(env);
      if(found) key = NULL;
    }
  }
  g_mutex_unlock(&export_ai_load_lock);
  g_free(key);
  if(!found) *err = g_strdup("face model is unavailable; verify the Faces model pack");
  return found;
}

static gboolean _faces_do(JsonObject *req, JsonBuilder *reply, char **err)
{
  if(json_object_get_boolean_member_with_default(req, "unload", FALSE))
  {
    g_mutex_lock(&export_ai_load_lock);
    _faces_unload_all();
    g_mutex_unlock(&export_ai_load_lock);
    return TRUE;
  }
  const char *root = json_object_get_string_member_with_default(req, "model_root", "");
  const char *id = json_object_get_string_member_with_default(req, "model_id", "");
  const char *file = json_object_get_string_member_with_default(req, "model_file", "");
  const char *input = json_object_get_string_member_with_default(req, "input", "");
  const char *output = json_object_get_string_member_with_default(req, "output", "");
  if(!g_path_is_absolute(root) || !g_path_is_absolute(input) || !g_path_is_absolute(output)
     || !_faces_plain(id) || !_faces_model_path(file))
  { *err = g_strdup("Invalid face model request"); return FALSE; }
  JsonArray *shape_json = json_object_has_member(req, "shape") ? json_object_get_array_member(req, "shape") : NULL;
  int64_t in_shape[4] = { 0 };
  const guint nd = shape_json ? json_array_get_length(shape_json) : 0;
  if(nd < 1 || nd > 4) { *err = g_strdup("Invalid face tensor shape"); return FALSE; }
  for(guint i = 0; i < nd; i++)
  {
    in_shape[i] = json_array_get_int_element(shape_json, i);
    if(in_shape[i] < 1 || in_shape[i] > 4096) { *err = g_strdup("Invalid face tensor shape"); return FALSE; }
  }
  const size_t count = dt_ai_tensor_element_count(in_shape, (int)nd);
  const size_t expected = count * sizeof(float);
  FILE *file_in = g_fopen(input, "rb");
  struct stat st;
  if(!file_in || fstat(fileno(file_in), &st) || !S_ISREG(st.st_mode) || st.st_size < 0
     || (uint64_t)st.st_size != expected || expected == 0 || expected > FACES_MAX_BYTES)
  { if(file_in) fclose(file_in); *err = g_strdup("Invalid face tensor file size or type"); return FALSE; }
  float *buffer = g_malloc(expected);
  const gboolean read_ok = fread(buffer, 1, expected, file_in) == expected && fgetc(file_in) == EOF && !ferror(file_in);
  fclose(file_in);
  if(!read_ok) { g_free(buffer); *err = g_strdup("Could not read the face tensor"); return FALSE; }
  uint32_t *bits = (uint32_t *)buffer;
  for(size_t i = 0; i < count; i++)
  {
    bits[i] = GUINT32_FROM_LE(bits[i]);
    float value; memcpy(&value, &bits[i], 4);
    if(!isfinite(value) || fabsf(value) > 1e4f) { g_free(buffer); *err = g_strdup("Invalid face tensor values"); return FALSE; }
  }
  dt_ai_context_t *ctx = _faces_load(root, id, file, err);
  if(!ctx) { g_free(buffer); return FALSE; }
  const int no = dt_ai_get_output_count(ctx);
  gboolean ok = dt_ai_get_input_count(ctx) == 1 && dt_ai_get_input_type(ctx, 0) == DT_AI_FLOAT && no >= 1 && no <= FACES_MAX_OUT;
  dt_ai_tensor_t input_t = { buffer, DT_AI_FLOAT, in_shape, (int)nd };
  dt_ai_tensor_t outputs[FACES_MAX_OUT] = { 0 };
  int64_t shapes[FACES_MAX_OUT][4] = { { 0 } };
  for(int i = 0; ok && i < no; i++)
  {
    const int rank = dt_ai_get_output_shape(ctx, i, shapes[i], 4);
    size_t n = rank >= 1 && rank <= 4 ? dt_ai_tensor_element_count(shapes[i], rank) : 0;
    if(dt_ai_get_output_type(ctx, i) != DT_AI_FLOAT || n == 0 || n > (16u << 20)) { ok = FALSE; break; }
    outputs[i] = (dt_ai_tensor_t){ g_malloc0(n * sizeof(float)), DT_AI_FLOAT, shapes[i], rank };
  }
  if(!ok) *err = g_strdup("Unsupported face model inputs, outputs, types or shapes");
  else if(dt_ai_run(ctx, &input_t, 1, outputs, no) < 0)
  { ok = FALSE; *err = g_strdup("CPU face inference failed"); }
  if(ok)
  {
    FILE *out = g_fopen(output, "wb");
    json_builder_set_member_name(reply, "outputs"); json_builder_begin_array(reply);
    for(int i = 0; i < no; i++)
    {
      const size_t n = dt_ai_tensor_element_count(shapes[i], outputs[i].ndim);
      float *data = outputs[i].data;
      for(size_t k = 0; k < n; k++)
        if(!isfinite(data[k])) ok = FALSE;
      for(size_t k = 0; out && ok && k < n; k++)
      {
        uint32_t v; memcpy(&v, &data[k], 4); v = GUINT32_TO_LE(v);
        if(fwrite(&v, 4, 1, out) != 1) ok = FALSE;
      }
      json_builder_begin_object(reply);
      json_builder_set_member_name(reply, "name"); json_builder_add_string_value(reply, dt_ai_get_output_name(ctx, i));
      json_builder_set_member_name(reply, "shape"); json_builder_begin_array(reply);
      for(int d = 0; d < outputs[i].ndim; d++) json_builder_add_int_value(reply, shapes[i][d]);
      json_builder_end_array(reply);
      json_builder_end_object(reply);
    }
    json_builder_end_array(reply);
    if(!out || fclose(out)) ok = FALSE;
    if(!ok) *err = g_strdup("face model produced an invalid output or the output file failed");
  }
  for(int i = 0; i < FACES_MAX_OUT; i++) g_free(outputs[i].data);
  g_free(buffer);
  return ok;
}
#else
static gboolean _faces_do(JsonObject *req, JsonBuilder *reply, char **err)
{ (void)req; (void)reply; *err = g_strdup("This engine was built without CPU ONNX support"); return FALSE; }
#endif

static void _faces_reply_error(FILE *out, JsonObject *req, const char *error)
{
  JsonBuilder *b = json_builder_new(); json_builder_begin_object(b);
  json_builder_set_member_name(b, "tag"); json_builder_add_int_value(b, json_object_get_int_member_with_default(req, "tag", 0));
  json_builder_set_member_name(b, "ok"); json_builder_add_boolean_value(b, FALSE);
  json_builder_set_member_name(b, "error"); json_builder_add_string_value(b, error);
  json_builder_end_object(b); engine_reply(out, b); g_object_unref(b);
}

static void _faces_run(gpointer data, gpointer user)
{
  JsonParser *parser = data; JsonObject *req = json_node_get_object(json_parser_get_root(parser));
  JsonBuilder *b = json_builder_new(); char *error = NULL;
  json_builder_begin_object(b);
  json_builder_set_member_name(b, "tag"); json_builder_add_int_value(b, json_object_get_int_member_with_default(req, "tag", 0));
  const gint64 t0 = g_get_monotonic_time();
  const gboolean ok = _faces_do(req, b, &error);
  json_builder_set_member_name(b, "ok"); json_builder_add_boolean_value(b, ok);
  engine_add_double(b, "ms", engine_ms_since(t0));
  if(!ok) { json_builder_set_member_name(b, "error"); json_builder_add_string_value(b, error ? error : "face inference failed"); }
  json_builder_end_object(b); engine_reply(user, b);
  g_free(error); g_object_unref(b); g_object_unref(parser);
  g_mutex_lock(&_faces_lock); _faces_pending--; g_mutex_unlock(&_faces_lock);
}

void faces_start(FILE *out)
{
  _faces_out = out;
  _faces_stopped = FALSE;
}

// Takes ownership of parser on every path. At most eight requests, including the active one.
void faces_queue(JsonParser *parser)
{
  JsonObject *req = json_node_get_object(json_parser_get_root(parser));
  g_mutex_lock(&_faces_lock);
  if(_faces_stopped || _faces_pending >= 8)
  { g_mutex_unlock(&_faces_lock); _faces_reply_error(_faces_out, req, "face worker is full or stopping"); g_object_unref(parser); return; }
  GError *error = NULL;
  if(!_faces_pool) _faces_pool = g_thread_pool_new(_faces_run, _faces_out, 1, TRUE, &error);
  if(!_faces_pool)
  {
    g_mutex_unlock(&_faces_lock);
    _faces_reply_error(_faces_out, req, error ? error->message : "could not start the face worker");
    g_clear_error(&error); g_object_unref(parser); return;
  }
  _faces_pending++;
  const gboolean pushed = g_thread_pool_push(_faces_pool, parser, &error);
  if(!pushed) _faces_pending--;
  g_mutex_unlock(&_faces_lock);
  if(!pushed)
  { _faces_reply_error(_faces_out, req, error ? error->message : "face queue failed"); g_clear_error(&error); g_object_unref(parser); }
}

void faces_wait(void)
{
  g_mutex_lock(&_faces_lock); _faces_stopped = TRUE; GThreadPool *pool = _faces_pool; _faces_pool = NULL; g_mutex_unlock(&_faces_lock);
  if(pool) g_thread_pool_free(pool, FALSE, TRUE);
#ifdef HAVE_AI
  g_mutex_lock(&export_ai_load_lock); _faces_unload_all(); g_mutex_unlock(&export_ai_load_lock);
#endif
}
