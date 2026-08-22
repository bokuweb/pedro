//! Turning a passage into a vector.
//!
//! A *static* embedding: one vector per token, looked up and averaged. There is
//! no transformer to run, so a book can be embedded on the machine that is
//! reading it, in about the time it takes to read the pages — which is the only
//! kind of embedding that belongs in an application with no server behind it.
//!
//! The model is `hotchpotch/static-embedding-japanese` (MIT), fetched by
//! `scripts/fetch-embedding.sh`. Everything works without it; this is what adds
//! searching by meaning rather than by word.
//!
//! Adapted from the author's ellisii-toolkit `embed-static-jp`, with
//! permission.

use std::path::{Path, PathBuf};

use memmap2::Mmap;
use safetensors::SafeTensors;
use tokenizers::Tokenizer;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("the embedding model could not be read: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Model(String),
}

/// The names the embedding table goes by, in the models this can load.
const TENSORS: [&str; 4] = [
    "embedding.weight",
    "embeddings.weight",
    "0_StaticEmbedding.embedding.weight",
    "weight",
];

/// A table of one vector per token, and the tokenizer that finds them.
pub struct Embedder {
    tokenizer: Tokenizer,
    table: Table,
}

impl Embedder {
    /// Loads the model from a directory holding `tokenizer.json` and
    /// `model.safetensors`.
    pub fn load(directory: &Path) -> Result<Self, EmbedError> {
        let tokenizer = Tokenizer::from_file(directory.join("tokenizer.json"))
            .map_err(|err| EmbedError::Model(format!("tokenizer.json: {err}")))?;
        let table = Table::load(&directory.join("model.safetensors"))?;

        tracing::info!(
            ?directory,
            vocabulary = table.vocabulary,
            dimensions = table.dimensions,
            "loaded the embedding model"
        );

        Ok(Self { tokenizer, table })
    }

    /// Loads the model from wherever it is, or reports that it is not there.
    ///
    /// The same search order as pdfium's: what the environment says, then what
    /// the fetch script writes, in any ancestor of the working directory or of
    /// the executable.
    pub fn find() -> Option<Self> {
        let path = model_path()?;

        match Self::load(&path) {
            Ok(embedder) => Some(embedder),
            Err(err) => {
                tracing::warn!(?err, ?path, "could not load the embedding model");
                None
            }
        }
    }

    /// How long the vectors are.
    pub fn dimensions(&self) -> usize {
        self.table.dimensions
    }

    /// The vector for one passage.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let encoded = self
            .tokenizer
            .encode(text, false)
            .map_err(|err| EmbedError::Model(format!("encode: {err}")))?;

        Ok(self.table.mean_of(encoded.get_ids()))
    }

    /// The vectors for many passages, in order.
    pub fn embed_all(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let encoded = self
            .tokenizer
            .encode_batch(texts.to_vec(), false)
            .map_err(|err| EmbedError::Model(format!("encode: {err}")))?;

        Ok(encoded
            .iter()
            .map(|encoded| self.table.mean_of(encoded.get_ids()))
            .collect())
    }
}

/// The embedding table, kept in the file it came from.
struct Table {
    /// The mapped file, which the rows point into.
    _file: Mmap,
    /// Where in the mapping the table starts, and how it is shaped.
    at: usize,
    vocabulary: usize,
    dimensions: usize,
    /// A copy, made only when the mapping is not aligned for `f32`.
    copied: Option<Vec<f32>>,
    /// The mapping, as bytes.
    bytes: *const u8,
    length: usize,
}

// The mapping is read-only and never moved after construction.
unsafe impl Send for Table {}
unsafe impl Sync for Table {}

impl Table {
    fn load(path: &Path) -> Result<Self, EmbedError> {
        let file = std::fs::File::open(path)?;
        // Mapped rather than read: the table is 128MB and only the rows of the
        // tokens actually used are ever touched, which the operating system is
        // better placed to decide than we are.
        let mapping = unsafe { Mmap::map(&file)? };

        let tensors = SafeTensors::deserialize(&mapping)
            .map_err(|err| EmbedError::Model(format!("safetensors: {err}")))?;
        let names: Vec<String> = tensors.names().into_iter().cloned().collect();
        let name = TENSORS
            .iter()
            .find(|wanted| names.iter().any(|name| name == *wanted))
            .map(|name| (*name).to_owned())
            .or_else(|| names.first().cloned())
            .ok_or_else(|| EmbedError::Model("the model holds no tensors".into()))?;

        let view = tensors
            .tensor(&name)
            .map_err(|err| EmbedError::Model(format!("tensor {name}: {err}")))?;
        let shape = view.shape();
        if shape.len() != 2 {
            return Err(EmbedError::Model(format!(
                "expected a table, got {shape:?}"
            )));
        }

        let (vocabulary, dimensions) = (shape[0], shape[1]);
        let data = view.data();
        if data.len() != vocabulary * dimensions * 4 {
            return Err(EmbedError::Model("the table is the wrong size".into()));
        }

        // Where the tensor sits inside the mapping, so the rows can be read
        // from it after the borrow ends.
        let at = data.as_ptr() as usize - mapping.as_ptr() as usize;
        let aligned = data.as_ptr().align_offset(align_of::<f32>()) == 0;
        let copied = (!aligned).then(|| {
            tracing::debug!("the embedding table is unaligned; copying it");
            data.as_chunks::<4>()
                .0
                .iter()
                .map(|four| f32::from_le_bytes(*four))
                .collect()
        });

        Ok(Self {
            bytes: mapping.as_ptr(),
            length: mapping.len(),
            _file: mapping,
            at,
            vocabulary,
            dimensions,
            copied,
        })
    }

    /// One token's vector.
    fn row(&self, token: usize) -> &[f32] {
        let from = token * self.dimensions;

        match &self.copied {
            Some(copied) => &copied[from..from + self.dimensions],
            None => {
                // Safe: the mapping outlives this, the tensor was checked to
                // hold `vocabulary * dimensions` floats, and the caller has
                // checked the token is inside the vocabulary.
                let floats = unsafe {
                    std::slice::from_raw_parts(
                        self.bytes.add(self.at) as *const f32,
                        (self.length - self.at) / 4,
                    )
                };
                &floats[from..from + self.dimensions]
            }
        }
    }

    /// The mean of the given tokens' vectors, normalised.
    ///
    /// Normalised so that a dot product is the cosine of the angle between two
    /// passages, which is what the vector index compares.
    fn mean_of(&self, tokens: &[u32]) -> Vec<f32> {
        let mut mean = vec![0.0_f32; self.dimensions];
        let mut counted = 0.0_f32;

        for token in tokens {
            let token = *token as usize;
            if token >= self.vocabulary {
                continue;
            }

            for (into, from) in mean.iter_mut().zip(self.row(token)) {
                *into += *from;
            }
            counted += 1.0;
        }

        if counted > 0.0 {
            for value in &mut mean {
                *value /= counted;
            }
        }

        normalise(&mut mean);
        mean
    }
}

/// Scales a vector to length one, leaving a vector of zeroes alone.
fn normalise(vector: &mut [f32]) {
    let length = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if length > 0.0 {
        for value in vector {
            *value /= length;
        }
    }
}

/// Where the model is, if it is anywhere.
fn model_path() -> Option<PathBuf> {
    if let Some(configured) = std::env::var_os("PEDRO_EMBEDDING_PATH") {
        return Some(PathBuf::from(configured));
    }

    let roots = std::env::current_dir().into_iter().chain(
        std::env::current_exe()
            .ok()
            .and_then(|executable| executable.parent().map(Path::to_path_buf)),
    );

    roots
        .flat_map(|root| {
            root.ancestors()
                .map(|ancestor| ancestor.join("vendor/embedding"))
                .collect::<Vec<_>>()
        })
        .find(|candidate| candidate.join("model.safetensors").is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalising_gives_a_vector_of_length_one() {
        let mut vector = vec![3.0, 4.0];
        normalise(&mut vector);

        assert!((vector.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-6);
    }

    /// A passage of nothing but unknown tokens has no direction, and dividing
    /// by its length would make it a vector of NaN.
    #[test]
    fn normalising_leaves_zero_alone() {
        let mut vector = vec![0.0, 0.0];
        normalise(&mut vector);

        assert_eq!(vector, vec![0.0, 0.0]);
    }

    #[test]
    fn the_model_is_looked_for_under_vendor() {
        // Only that the search names the place the fetch script writes to;
        // whether it is there depends on whether anyone ran it.
        if let Some(path) = model_path() {
            assert!(path.ends_with("vendor/embedding"), "{path:?}");
        }
    }
}
