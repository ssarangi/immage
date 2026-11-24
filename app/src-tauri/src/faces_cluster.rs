// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Who is who, from SFace embeddings (L2-normalised, 128-d, cosine = dot product).
//!
//! `suggest` names a face after the person whose confirmed face it resembles most (best exemplar,
//! not a mean, so a person seen young and old still matches). `cluster` groups the faces nobody
//! has named yet: average-link agglomerative clustering, which, unlike the old app's single link,
//! does not chain two people through one ambiguous face, because a merge needs the clusters to
//! agree on average. It is not quadratic over the library: candidate pairs come from the
//! embeddings' random-hyperplane buckets (exact all-pairs only up to `exact_limit` faces), the
//! pairs at or above the threshold split the faces into components (an average-link cluster is
//! always inside one component of its own threshold), and the dense agglomeration runs per
//! component only, capped in size.
use crate::faces_detect::cosine;

/// SFace: a face is suggested for a person at or above this cosine to one of their faces.
pub const SUGGEST_SFACE: f32 = 0.45;
/// SFace: the cosine at which two unnamed faces may share a cluster (average over the whole cluster).
pub const CLUSTER_SFACE: f32 = 0.48;

#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub threshold: f32,
    pub min_size: usize,
    pub max_size: usize,
    /// Up to this many faces every pair is compared; beyond it only faces sharing a hash bucket are.
    pub exact_limit: usize,
    /// A component bigger than this is split by raising its threshold before the dense step.
    pub component_cap: usize,
}
impl Default for Params {
    fn default() -> Self {
        Params { threshold: CLUSTER_SFACE, min_size: 2, max_size: 2000, exact_limit: 2500, component_cap: 3000 }
    }
}

// A cap of 200 once split one person with 277 photos into two groups (the owner's data: centroid cosine
// 0.91 between the halves), so a group may now hold as many faces as one dense component does.

/// The person whose best exemplar is closest to `face`, if that is at or above `min`. `exemplars` is
/// (person id, embedding) over the confirmed faces of every active person; `skip` excludes people the
/// user already said this face is not.
pub fn suggest(face: &[f32], exemplars: &[(i64, &[f32])], min: f32, skip: &[i64]) -> Option<(i64, f32)> {
    let mut best: Option<(i64, f32)> = None;
    for (person, emb) in exemplars {
        if skip.contains(person) {
            continue;
        }
        let s = cosine(face, emb);
        if s >= min && best.is_none_or(|(_, b)| s > b || (s == b && *person < best.unwrap().0)) {
            best = Some((*person, s));
        }
    }
    best
}

struct Dsu {
    parent: Vec<usize>,
    size: Vec<usize>,
}
impl Dsu {
    fn new(n: usize) -> Self {
        Dsu { parent: (0..n).collect(), size: vec![1; n] }
    }
    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }
    fn union(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] += self.size[b];
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Approximately normal (sum of four uniforms, centred).
    fn gauss(&mut self) -> f32 {
        (self.unit() + self.unit() + self.unit() + self.unit() - 2.0) * 1.7320508
    }
}

const TABLES: usize = 8;
const BITS: usize = 8;
const BUCKET_WINDOW: usize = 300;

/// Pairs (i, j, cosine) at or above `min`, all of them for few faces, those sharing a bucket otherwise.
pub fn candidate_edges(embs: &[&[f32]], min: f32, exact_limit: usize) -> Vec<(u32, u32, f32)> {
    let n = embs.len();
    let mut edges = Vec::new();
    if n <= exact_limit {
        for i in 0..n {
            for j in i + 1..n {
                let s = cosine(embs[i], embs[j]);
                if s >= min {
                    edges.push((i as u32, j as u32, s));
                }
            }
        }
        return edges;
    }
    let dims = embs[0].len();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..TABLES {
        let planes: Vec<Vec<f32>> = (0..BITS).map(|_| (0..dims).map(|_| rng.gauss()).collect()).collect();
        let mut buckets: std::collections::HashMap<u16, Vec<u32>> = std::collections::HashMap::new();
        for (i, e) in embs.iter().enumerate() {
            let mut code = 0u16;
            for (b, p) in planes.iter().enumerate() {
                if cosine(e, p) >= 0.0 {
                    code |= 1 << b;
                }
            }
            buckets.entry(code).or_default().push(i as u32);
        }
        for members in buckets.values() {
            for (a, &i) in members.iter().enumerate() {
                for &j in members.iter().skip(a + 1).take(BUCKET_WINDOW) {
                    let (lo, hi) = (i.min(j), i.max(j));
                    let s = cosine(embs[lo as usize], embs[hi as usize]);
                    if s >= min && seen.insert((lo, hi)) {
                        edges.push((lo, hi, s));
                    }
                }
            }
        }
    }
    edges
}

/// Average-link agglomerative clustering of one component (dense). Returns clusters of indices into `members`.
fn average_link(embs: &[&[f32]], members: &[usize], threshold: f32, max_size: usize) -> Vec<Vec<usize>> {
    let m = members.len();
    let mut sim = vec![f32::NEG_INFINITY; m * m];
    for a in 0..m {
        for b in a + 1..m {
            let s = cosine(embs[members[a]], embs[members[b]]);
            sim[a * m + b] = s;
            sim[b * m + a] = s;
        }
    }
    let mut size = vec![1usize; m];
    let mut alive = vec![true; m];
    let mut groups: Vec<Vec<usize>> = (0..m).map(|i| vec![members[i]]).collect();
    // best partner of each row
    let best_of = |sim: &[f32], alive: &[bool], i: usize| -> (usize, f32) {
        let mut best = (usize::MAX, f32::NEG_INFINITY);
        for j in 0..m {
            if j != i && alive[j] && sim[i * m + j] > best.1 {
                best = (j, sim[i * m + j]);
            }
        }
        best
    };
    let mut best: Vec<(usize, f32)> = (0..m).map(|i| best_of(&sim, &alive, i)).collect();
    loop {
        let (mut a, mut top) = (usize::MAX, threshold);
        for i in 0..m {
            if alive[i] && best[i].0 != usize::MAX && best[i].1 >= top {
                a = i;
                top = best[i].1;
            }
        }
        if a == usize::MAX {
            break;
        }
        let b = best[a].0;
        if size[a] + size[b] > max_size {
            // too big to merge: this pair is never considered again
            sim[a * m + b] = f32::NEG_INFINITY;
            sim[b * m + a] = f32::NEG_INFINITY;
            best[a] = best_of(&sim, &alive, a);
            best[b] = best_of(&sim, &alive, b);
            continue;
        }
        // merge b into a (Lance-Williams for average link)
        let (sa, sb) = (size[a] as f32, size[b] as f32);
        for c in 0..m {
            if c == a || c == b || !alive[c] {
                continue;
            }
            let (x, y) = (sim[a * m + c], sim[b * m + c]);
            let s = if x == f32::NEG_INFINITY || y == f32::NEG_INFINITY { f32::NEG_INFINITY } else { (sa * x + sb * y) / (sa + sb) };
            sim[a * m + c] = s;
            sim[c * m + a] = s;
        }
        alive[b] = false;
        size[a] += size[b];
        let moved = std::mem::take(&mut groups[b]);
        groups[a].extend(moved);
        best[a] = best_of(&sim, &alive, a);
        for c in 0..m {
            if alive[c] && c != a && (best[c].0 == a || best[c].0 == b) {
                best[c] = best_of(&sim, &alive, c);
            } else if alive[c] && c != a && sim[c * m + a] > best[c].1 {
                best[c] = (a, sim[c * m + a]);
            }
        }
    }
    (0..m).filter(|&i| alive[i]).map(|i| std::mem::take(&mut groups[i])).collect()
}

/// Clusters of at least `min_size` faces (indices into `embs`), each sorted, the list sorted by first index.
pub fn cluster(embs: &[&[f32]], p: &Params) -> Vec<Vec<usize>> {
    let n = embs.len();
    if n < 2 {
        return Vec::new();
    }
    let mut edges = candidate_edges(embs, p.threshold, p.exact_limit);
    edges.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut components = components_at(n, &edges, p.threshold);
    let mut out: Vec<Vec<usize>> = Vec::new();
    // an oversize component is split by raising the threshold; the rest is handled as it comes
    let mut guard = 0;
    while let Some(c) = components.pop() {
        if c.len() < 2 {
            continue;
        }
        if c.len() > p.component_cap && guard < 64 {
            guard += 1;
            let mut t = p.threshold;
            let set: std::collections::HashSet<usize> = c.iter().copied().collect();
            let inner: Vec<(u32, u32, f32)> = edges.iter().copied().filter(|e| set.contains(&(e.0 as usize))).collect();
            let mut pieces = vec![c.clone()];
            while t < 0.95 {
                t += 0.02;
                pieces = components_at_subset(&c, &inner, t);
                if pieces.iter().all(|x| x.len() <= p.component_cap) {
                    break;
                }
            }
            if pieces.len() == 1 && pieces[0].len() == c.len() {
                // no threshold splits it (a block of near-identical faces): cut it into chunks
                for chunk in c.chunks(p.max_size) {
                    if chunk.len() >= p.min_size {
                        out.push(chunk.to_vec());
                    }
                }
            } else {
                for piece in pieces {
                    if piece.len() > p.component_cap {
                        for chunk in piece.chunks(p.max_size) {
                            if chunk.len() >= p.min_size {
                                out.push(chunk.to_vec());
                            }
                        }
                    } else if piece.len() >= 2 {
                        out.extend(average_link(embs, &piece, p.threshold, p.max_size));
                    }
                }
            }
            continue;
        }
        out.extend(average_link(embs, &c, p.threshold, p.max_size));
    }
    let mut out: Vec<Vec<usize>> = out
        .into_iter()
        .filter(|c| c.len() >= p.min_size)
        .map(|mut c| {
            c.sort_unstable();
            c
        })
        .collect();
    out.sort_by_key(|c| c[0]);
    out
}

fn components_at(n: usize, edges: &[(u32, u32, f32)], t: f32) -> Vec<Vec<usize>> {
    let mut dsu = Dsu::new(n);
    for &(a, b, s) in edges {
        if s >= t {
            dsu.union(a as usize, b as usize);
        }
    }
    let mut by_root: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for i in 0..n {
        let r = dsu.find(i);
        by_root.entry(r).or_default().push(i);
    }
    by_root.into_values().collect()
}

fn components_at_subset(members: &[usize], edges: &[(u32, u32, f32)], t: f32) -> Vec<Vec<usize>> {
    let index: std::collections::HashMap<usize, usize> = members.iter().enumerate().map(|(k, &i)| (i, k)).collect();
    let mut dsu = Dsu::new(members.len());
    for &(a, b, s) in edges {
        if s >= t {
            if let (Some(&x), Some(&y)) = (index.get(&(a as usize)), index.get(&(b as usize))) {
                dsu.union(x, y);
            }
        }
    }
    let mut by_root: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for (k, &i) in members.iter().enumerate() {
        let r = dsu.find(k);
        by_root.entry(r).or_default().push(i);
    }
    by_root.into_values().collect()
}

fn mean_vec(embs: &[&[f32]], members: &[usize]) -> Vec<f32> {
    let mut v = vec![0f32; embs[0].len()];
    for &i in members {
        for (a, b) in v.iter_mut().zip(embs[i]) {
            *a += b;
        }
    }
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

/// The score of a face against a group: the better of its cosine to the group's centroid and the mean
/// of its three closest members (a person in another pose is close to some of their faces, not to the mean).
fn group_score(face: &[f32], embs: &[&[f32]], members: &[usize], centroid: &[f32]) -> f32 {
    let mut sims: Vec<f32> = members.iter().map(|&m| cosine(face, embs[m])).collect();
    sims.sort_by(|a, b| b.total_cmp(a));
    let k = sims.len().min(3).max(1);
    let top: f32 = sims.iter().take(k).sum::<f32>() / k as f32;
    top.max(cosine(face, centroid))
}

/// Joins groups whose centroids agree (cosine at or above `threshold`), most alike first, until none do.
pub fn merge_groups(embs: &[&[f32]], groups: Vec<Vec<usize>>, threshold: f32) -> Vec<Vec<usize>> {
    // the merge pass is a matrix over the groups: only the biggest few thousand take part
    const LIMIT: usize = 3000;
    let mut groups = groups;
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let rest = if groups.len() > LIMIT { groups.split_off(LIMIT) } else { Vec::new() };
    let k = groups.len();
    let mut cents: Vec<Vec<f32>> = groups.iter().map(|g| mean_vec(embs, g)).collect();
    let mut alive = vec![true; k];
    let mut sim = vec![f32::NEG_INFINITY; k * k];
    for a in 0..k {
        for b in a + 1..k {
            let s = cosine(&cents[a], &cents[b]);
            sim[a * k + b] = s;
            sim[b * k + a] = s;
        }
    }
    let best_of = |sim: &[f32], alive: &[bool], i: usize| -> (usize, f32) {
        let mut best = (usize::MAX, f32::NEG_INFINITY);
        for j in 0..k {
            if j != i && alive[j] && sim[i * k + j] > best.1 {
                best = (j, sim[i * k + j]);
            }
        }
        best
    };
    let mut best: Vec<(usize, f32)> = (0..k).map(|i| best_of(&sim, &alive, i)).collect();
    loop {
        let (mut a, mut top) = (usize::MAX, threshold);
        for i in 0..k {
            if alive[i] && best[i].0 != usize::MAX && best[i].1 >= top {
                a = i;
                top = best[i].1;
            }
        }
        if a == usize::MAX {
            break;
        }
        let b = best[a].0;
        let moved = std::mem::take(&mut groups[b]);
        groups[a].extend(moved);
        alive[b] = false;
        cents[a] = mean_vec(embs, &groups[a]);
        for c in 0..k {
            if alive[c] && c != a {
                let s = cosine(&cents[a], &cents[c]);
                sim[a * k + c] = s;
                sim[c * k + a] = s;
            }
        }
        best[a] = best_of(&sim, &alive, a);
        for c in 0..k {
            if alive[c] && c != a {
                if best[c].0 == a || best[c].0 == b {
                    best[c] = best_of(&sim, &alive, c);
                } else if sim[c * k + a] > best[c].1 {
                    best[c] = (a, sim[c * k + a]);
                }
            }
        }
    }
    let mut out: Vec<Vec<usize>> = (0..k).filter(|&i| alive[i]).map(|i| std::mem::take(&mut groups[i])).collect();
    out.extend(rest);
    for g in out.iter_mut() {
        g.sort_unstable();
    }
    out.sort_by_key(|g| g[0]);
    out
}

/// Groups unnamed faces the way the page needs them: seeds first, the rest attached, then a merge pass.
///
/// 1. Only `seed` faces (big enough, sharp enough, facing the camera enough) are clustered, by average link
///    at `th.cluster`, so a blurry or tiny face cannot pull two people together or start a group of its own.
/// 2. Every other face joins the group it matches best (`group_score` at or above `th.attach`) when that group
///    beats the second best by 0.04; otherwise it stays loose.
/// 3. Groups whose centroids reach `th.merge` are joined.
/// Returns the groups as indices into `embs`, each of at least `min_size` faces.
pub fn group_faces(embs: &[&[f32]], seed: &[bool], th: &crate::faces_models::Thresholds, params: &Params) -> Vec<Vec<usize>> {
    let seeds: Vec<usize> = (0..embs.len()).filter(|&i| seed[i]).collect();
    let seed_embs: Vec<&[f32]> = seeds.iter().map(|&i| embs[i]).collect();
    let mut groups: Vec<Vec<usize>> = cluster(&seed_embs, &Params { threshold: th.cluster, ..*params }).into_iter().map(|g| g.into_iter().map(|k| seeds[k]).collect()).collect();
    let cents: Vec<Vec<f32>> = groups.iter().map(|g| mean_vec(embs, g)).collect();
    let mut joined: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for i in (0..embs.len()).filter(|&i| !seed[i]) {
        let mut scores: Vec<(f32, usize)> = groups.iter().enumerate().map(|(g, m)| (group_score(embs[i], embs, m, &cents[g]), g)).collect();
        scores.sort_by(|a, b| b.0.total_cmp(&a.0));
        if let Some(&(best, g)) = scores.first() {
            let second = scores.get(1).map_or(f32::NEG_INFINITY, |s| s.0);
            if best >= th.attach && best - second >= 0.04 {
                joined[g].push(i);
            }
        }
    }
    for (g, extra) in joined.into_iter().enumerate() {
        groups[g].extend(extra);
    }
    let merged = if groups.len() > 1 { merge_groups(embs, groups, th.merge) } else { groups };
    merged.into_iter().filter(|g| g.len() >= params.min_size).collect()
}

/// A pair the page may offer to join: two groups, two people, or a group and a person.
#[derive(Clone, Debug, PartialEq, serde::Serialize, specta::Type)]
pub struct MergeSuggestion {
    /// "groups", "group_person" (a group that looks like a named person) or "people"
    pub kind: String,
    /// The first side: a group id, or a person id for "people". For "group_person" it is the group.
    pub a: i64,
    /// The second side: a group id, or a person id.
    pub b: i64,
    pub similarity: f32,
    /// "high" (at the merge threshold), "medium" or "low"
    pub confidence: String,
}

/// Pairs worth showing, strongest first, at most `limit`. `groups` and `people` are (id, member embeddings); a person's
/// members are all their confirmed faces, so a group is compared with every one of them.
pub fn merge_suggestions(groups: &[(i64, Vec<&[f32]>)], people: &[(i64, Vec<&[f32]>)], th: &crate::faces_models::Thresholds, limit: usize) -> Vec<MergeSuggestion> {
    let centroid = |m: &[&[f32]]| -> Vec<f32> {
        let idx: Vec<usize> = (0..m.len()).collect();
        mean_vec(m, &idx)
    };
    let confidence = |s: f32| if s >= th.merge { "high" } else if s >= (th.merge + th.suggest) / 2.0 { "medium" } else { "low" }.to_string();
    let gc: Vec<Vec<f32>> = groups.iter().map(|g| centroid(&g.1)).collect();
    let pc: Vec<Vec<f32>> = people.iter().map(|p| centroid(&p.1)).collect();
    let mut out = Vec::new();
    for a in 0..groups.len() {
        for b in a + 1..groups.len() {
            let s = cosine(&gc[a], &gc[b]);
            if s >= th.suggest {
                out.push(MergeSuggestion { kind: "groups".into(), a: groups[a].0, b: groups[b].0, similarity: s, confidence: confidence(s) });
            }
        }
        for (p, person) in people.iter().enumerate() {
            // the person's closest faces to the group's centroid, or their centroid: whichever agrees more
            let mut sims: Vec<f32> = person.1.iter().map(|e| cosine(e, &gc[a])).collect();
            sims.sort_by(|x, y| y.total_cmp(x));
            let k = sims.len().min(3).max(1);
            let top = sims.iter().take(k).sum::<f32>() / k as f32;
            let s = top.max(cosine(&pc[p], &gc[a]));
            if s >= th.suggest {
                out.push(MergeSuggestion { kind: "group_person".into(), a: groups[a].0, b: person.0, similarity: s, confidence: confidence(s) });
            }
        }
    }
    for a in 0..people.len() {
        for b in a + 1..people.len() {
            let s = cosine(&pc[a], &pc[b]);
            if s >= th.suggest {
                out.push(MergeSuggestion { kind: "people".into(), a: people[a].0, b: people[b].0, similarity: s, confidence: confidence(s) });
            }
        }
    }
    out.sort_by(|x, y| y.similarity.total_cmp(&x.similarity));
    out.truncate(limit);
    out
}

/// The face of a cluster closest to the others on average (what the page shows for the group).
pub fn medoid(embs: &[&[f32]], cluster: &[usize]) -> usize {
    let mut best = (cluster[0], f32::NEG_INFINITY);
    for &i in cluster {
        let total: f32 = cluster.iter().filter(|&&j| j != i).map(|&j| cosine(embs[i], embs[j])).sum();
        if total > best.1 {
            best = (i, total);
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(mut v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
    fn random_unit(rng: &mut Rng, dims: usize) -> Vec<f32> {
        unit((0..dims).map(|_| rng.gauss()).collect())
    }
    /// `count` noisy copies of `center`; `noise` is relative to the centre's length.
    fn identity(rng: &mut Rng, center: &[f32], count: usize, noise: f32) -> Vec<Vec<f32>> {
        (0..count).map(|_| unit(center.iter().map(|c| c + noise * rng.gauss() / (center.len() as f32).sqrt()).collect())).collect()
    }
    fn refs(v: &[Vec<f32>]) -> Vec<&[f32]> {
        v.iter().map(Vec::as_slice).collect()
    }
    fn purity(clusters: &[Vec<usize>], truth: &[usize]) -> f32 {
        let total: usize = clusters.iter().map(Vec::len).sum();
        let right: usize = clusters
            .iter()
            .map(|c| {
                let mut counts = std::collections::HashMap::new();
                for &i in c {
                    *counts.entry(truth[i]).or_insert(0usize) += 1;
                }
                counts.values().copied().max().unwrap_or(0)
            })
            .sum();
        right as f32 / total.max(1) as f32
    }

    #[test]
    fn suggest_takes_the_best_exemplar_over_the_threshold_and_respects_rejections() {
        let a1 = [1.0f32, 0.0];
        let a2 = [0.6f32, 0.8];
        let b1 = [0.0f32, 1.0];
        let ex: Vec<(i64, &[f32])> = vec![(1, &a1), (1, &a2), (2, &b1)];
        let face = [0.8f32, 0.6];
        // best exemplar of person 1 is a2: 0.96; person 2: 0.6
        assert_eq!(suggest(&face, &ex, SUGGEST_SFACE, &[]).map(|s| s.0), Some(1));
        assert_eq!(suggest(&face, &ex, SUGGEST_SFACE, &[1]).map(|s| s.0), Some(2), "not person 1 says the user");
        assert_eq!(suggest(&face, &ex, 0.99, &[]), None);
        assert_eq!(suggest(&face, &[], SUGGEST_SFACE, &[]), None);
        // a tie goes to the lower id, deterministically
        let c1 = [1.0f32, 0.0];
        let tie: Vec<(i64, &[f32])> = vec![(7, &c1), (3, &c1)];
        assert_eq!(suggest(&c1, &tie, 0.1, &[]).map(|s| s.0), Some(3));
    }

    #[test]
    fn separate_people_form_separate_clusters_and_strangers_stay_out() {
        let mut rng = Rng(7);
        let (ca, cb, cc) = (random_unit(&mut rng, 128), random_unit(&mut rng, 128), random_unit(&mut rng, 128));
        let mut faces = identity(&mut rng, &ca, 12, 0.6);
        faces.extend(identity(&mut rng, &cb, 9, 0.6));
        faces.extend(identity(&mut rng, &cc, 6, 0.6));
        faces.push(random_unit(&mut rng, 128)); // a stranger
        let truth: Vec<usize> = (0..12).map(|_| 0).chain((0..9).map(|_| 1)).chain((0..6).map(|_| 2)).chain([3]).collect();
        let clusters = cluster(&refs(&faces), &Params::default());
        assert_eq!(clusters.len(), 3, "{clusters:?}");
        assert!(purity(&clusters, &truth) > 0.99);
        assert!(clusters.iter().all(|c| !c.contains(&27)), "the stranger is in no cluster");
    }

    #[test]
    fn average_link_does_not_chain_two_people_through_a_bridge_face() {
        let mut rng = Rng(11);
        let ca = random_unit(&mut rng, 128);
        let cb = random_unit(&mut rng, 128);
        let mut faces = identity(&mut rng, &ca, 10, 0.5);
        faces.extend(identity(&mut rng, &cb, 10, 0.5));
        // a face halfway between the two people: close to both, so single link would join them
        faces.push(unit(ca.iter().zip(&cb).map(|(a, b)| a + b).collect()));
        let r = refs(&faces);
        let single = components_at(r.len(), &{
            let mut e = candidate_edges(&r, CLUSTER_SFACE, 10_000);
            e.sort_by(|a, b| b.2.total_cmp(&a.2));
            e
        }, CLUSTER_SFACE);
        assert!(single.iter().any(|c| c.len() > 15), "single link chains them: {:?}", single.iter().map(Vec::len).collect::<Vec<_>>());
        let clusters = cluster(&r, &Params::default());
        let truth: Vec<usize> = (0..10).map(|_| 0).chain((0..10).map(|_| 1)).chain([2]).collect();
        assert!(clusters.iter().all(|c| c.len() <= 11), "average link keeps them apart: {:?}", clusters.iter().map(Vec::len).collect::<Vec<_>>());
        assert!(purity(&clusters, &truth) > 0.9);
        assert!(clusters.len() >= 2);
    }

    #[test]
    fn clusters_are_capped_in_size() {
        let mut rng = Rng(3);
        let c = random_unit(&mut rng, 128);
        let faces = identity(&mut rng, &c, 50, 0.3);
        let p = Params { max_size: 20, ..Params::default() };
        let clusters = cluster(&refs(&faces), &p);
        assert!(clusters.iter().all(|c| c.len() <= 20), "{:?}", clusters.iter().map(Vec::len).collect::<Vec<_>>());
        assert!(clusters.iter().map(Vec::len).sum::<usize>() >= 40);
    }

    #[test]
    fn the_hashed_path_finds_the_same_people_as_the_exact_one() {
        let mut rng = Rng(21);
        let mut faces = Vec::new();
        let mut truth = Vec::new();
        for person in 0..40 {
            let c = random_unit(&mut rng, 128);
            let k = 6 + (person % 5);
            faces.extend(identity(&mut rng, &c, k, 0.6));
            truth.extend(std::iter::repeat_n(person, k));
        }
        let r = refs(&faces);
        let exact = cluster(&r, &Params::default());
        let hashed = cluster(&r, &Params { exact_limit: 10, ..Params::default() });
        assert!(purity(&exact, &truth) > 0.99 && purity(&hashed, &truth) > 0.99);
        let grouped = |c: &[Vec<usize>]| c.iter().map(Vec::len).sum::<usize>();
        assert!(grouped(&hashed) as f32 >= 0.9 * grouped(&exact) as f32, "{} vs {}", grouped(&hashed), grouped(&exact));
    }

    #[test]
    fn hashed_clustering_scales_to_many_faces() {
        let mut rng = Rng(5);
        let mut faces = Vec::new();
        for _ in 0..800 {
            let c = random_unit(&mut rng, 128);
            faces.extend(identity(&mut rng, &c, 5, 0.6));
        }
        // 4000 faces; exact all-pairs would be 8M dots, the hashed path far fewer
        let started = std::time::Instant::now();
        let clusters = cluster(&refs(&faces), &Params { exact_limit: 100, ..Params::default() });
        assert!(clusters.len() > 600, "{}", clusters.len());
        assert!(started.elapsed().as_secs() < 30);
    }

    #[test]
    fn an_oversize_component_is_split_and_a_block_of_duplicates_is_chunked() {
        let mut rng = Rng(9);
        let c = random_unit(&mut rng, 128);
        let faces: Vec<Vec<f32>> = (0..300).map(|_| c.clone()).collect();
        let p = Params { component_cap: 100, max_size: 80, ..Params::default() };
        let clusters = cluster(&refs(&faces), &p);
        assert!(clusters.iter().all(|c| c.len() <= 80));
        assert_eq!(clusters.iter().map(Vec::len).sum::<usize>(), 300);
    }

    fn sface() -> crate::faces_models::Thresholds {
        crate::faces_models::thresholds(crate::faces_models::Recognizer::Sface)
    }

    #[test]
    fn a_person_with_hundreds_of_faces_is_one_group_not_two() {
        let mut rng = Rng(31);
        let c = random_unit(&mut rng, 128);
        let faces = identity(&mut rng, &c, 260, 0.6);
        let clusters = cluster(&refs(&faces), &Params::default());
        assert_eq!(clusters.len(), 1, "{:?}", clusters.iter().map(Vec::len).collect::<Vec<_>>());
        assert_eq!(clusters[0].len(), 260);
    }

    #[test]
    fn only_quality_faces_seed_groups_and_the_rest_attach_or_stay_loose() {
        let mut rng = Rng(41);
        let (ca, cb, stranger) = (random_unit(&mut rng, 128), random_unit(&mut rng, 128), random_unit(&mut rng, 128));
        let mut faces = identity(&mut rng, &ca, 12, 0.6);
        faces.extend(identity(&mut rng, &cb, 10, 0.6));
        let mut seed = vec![true; 22];
        // six poor faces of A (heavier noise), four of a person with no seed group, one lone poor face
        faces.extend(identity(&mut rng, &ca, 6, 1.0));
        faces.extend(identity(&mut rng, &stranger, 4, 0.3));
        faces.push(random_unit(&mut rng, 128));
        seed.extend(std::iter::repeat_n(false, 11));
        let groups = group_faces(&refs(&faces), &seed, &sface(), &Params::default());
        assert_eq!(groups.len(), 2, "{:?}", groups.iter().map(Vec::len).collect::<Vec<_>>());
        let a = groups.iter().find(|g| g.contains(&0)).unwrap();
        let attached = (22..28).filter(|i| a.contains(i)).count();
        assert!(attached >= 4, "the poor faces of A joined A's group: {attached} of 6");
        assert!(groups.iter().all(|g| g.iter().all(|&i| !(28..33).contains(&i))), "poor faces of nobody start no group and join none");
        assert!(!a.iter().any(|i| (10..22).contains(i) && *i >= 12), "B's faces are not in A's group");
    }

    #[test]
    fn the_merge_pass_joins_pieces_of_one_person_and_leaves_two_people_apart() {
        let mut rng = Rng(51);
        let (ca, cb) = (random_unit(&mut rng, 128), random_unit(&mut rng, 128));
        let mut faces = identity(&mut rng, &ca, 30, 0.6);
        faces.extend(identity(&mut rng, &cb, 30, 0.6));
        let r = refs(&faces);
        // forced into small pieces by a tiny size cap
        let pieces = cluster(&r, &Params { max_size: 8, ..Params::default() });
        assert!(pieces.len() >= 6, "{}", pieces.len());
        let merged = merge_groups(&r, pieces, sface().merge);
        assert_eq!(merged.len(), 2, "{:?}", merged.iter().map(Vec::len).collect::<Vec<_>>());
        assert!(merged.iter().all(|g| g.len() == 30 && (g[0] < 30) == g.iter().all(|&i| i < 30)));
        // two different people: the best centroid cosine is far below the merge threshold
        let two = vec![(0..30).collect::<Vec<_>>(), (30..60).collect::<Vec<_>>()];
        assert_eq!(merge_groups(&r, two, sface().merge).len(), 2);
    }

    #[test]
    fn pose_like_noise_splits_a_person_into_seeds_that_the_merge_pass_rejoins() {
        // frontal faces and 'profile' faces of one person: the profile ones share a pose offset, so they
        // are close to each other (0.7) and only moderately close to the frontal ones (0.5)
        let mut rng = Rng(61);
        let c = random_unit(&mut rng, 128);
        let pose = random_unit(&mut rng, 128);
        let profile_center = unit(c.iter().zip(&pose).map(|(a, b)| a + 0.9 * b).collect());
        let mut faces = identity(&mut rng, &c, 15, 0.4);
        faces.extend(identity(&mut rng, &profile_center, 15, 0.4));
        let other = random_unit(&mut rng, 128);
        faces.extend(identity(&mut rng, &other, 15, 0.4));
        let seed = vec![true; faces.len()];
        let groups = group_faces(&refs(&faces), &seed, &sface(), &Params::default());
        let of_person = |g: &Vec<usize>| g.iter().all(|&i| i < 30);
        let person_groups: Vec<_> = groups.iter().filter(|g| of_person(g)).collect();
        assert!(groups.iter().all(|g| of_person(g) || g.iter().all(|&i| i >= 30)), "no group mixes the two people");
        assert_eq!(person_groups.len(), 1, "frontal and profile faces of the person are one group: {:?}", groups.iter().map(Vec::len).collect::<Vec<_>>());
    }

    #[test]
    fn suggestions_pair_alike_groups_and_name_a_group_after_a_person() {
        let mut rng = Rng(71);
        let (ca, cb, cc) = (random_unit(&mut rng, 128), random_unit(&mut rng, 128), random_unit(&mut rng, 128));
        let a1 = identity(&mut rng, &ca, 8, 0.5);
        let a2 = identity(&mut rng, &ca, 8, 0.5);
        let b = identity(&mut rng, &cb, 8, 0.5);
        let named = identity(&mut rng, &cc, 10, 0.5);
        let c_group = identity(&mut rng, &cc, 5, 0.5);
        let groups = vec![(1, refs(&a1)), (2, refs(&a2)), (3, refs(&b)), (4, refs(&c_group))];
        let people = vec![(9, refs(&named))];
        let s = merge_suggestions(&groups, &people, &sface(), 50);
        let find = |kind: &str, a: i64, b: i64| s.iter().find(|x| x.kind == kind && x.a == a && x.b == b);
        assert!(find("groups", 1, 2).is_some(), "{s:?}");
        assert_eq!(find("groups", 1, 2).unwrap().confidence, "high");
        assert!(find("group_person", 4, 9).is_some(), "the group looks like the named person");
        assert!(find("groups", 1, 3).is_none() && find("group_person", 3, 9).is_none() && find("group_person", 1, 9).is_none());
        assert!(s.windows(2).all(|w| w[0].similarity >= w[1].similarity), "strongest first");
        assert_eq!(merge_suggestions(&groups, &people, &sface(), 1).len(), 1);
        assert!(merge_suggestions(&[], &[], &sface(), 5).is_empty());
    }

    #[test]
    fn medoid_is_the_most_central_face() {
        let a = [1.0f32, 0.0];
        let b = unit(vec![0.9, 0.3]);
        let c = [0.8f32, 0.6];
        let embs: Vec<&[f32]> = vec![&a, &b, &c];
        assert_eq!(medoid(&embs, &[0, 1, 2]), 1);
    }

    #[test]
    fn no_faces_or_one_face_is_no_cluster() {
        assert!(cluster(&[], &Params::default()).is_empty());
        let a = [1.0f32, 0.0];
        assert!(cluster(&[&a], &Params::default()).is_empty());
    }
}
