/*!
Scratch filesystem paths for tests.

Tests here write real archives and extract real trees, so they need somewhere on
disk. Cleaning that up with a trailing `remove_dir_all` at the end of the test
only runs when the test passes: a failing assertion returns early and leaves the
directory behind, so the run that most needs a clean slate is the one that
pollutes it. `ScratchPath` removes itself on drop instead, which happens on the
failure path too.
*/

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A temp-directory path that deletes itself when it goes out of scope.
pub struct ScratchPath(PathBuf);

impl ScratchPath {
    /// An unused path under the system temp dir, named for `tag`.
    ///
    /// Unique per process *and* per call: tests in one binary run concurrently,
    /// so a name built from the pid alone collides whenever two tests derive the
    /// same one.
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let seq = NEXT.fetch_add(1, Ordering::Relaxed);
        let scratch =
            Self(std::env::temp_dir().join(format!("dat3_{tag}_{}_{seq}", std::process::id())));
        // A previous run killed before its drop ran could have left this behind.
        scratch.remove();
        scratch
    }

    /// The same, with the directory created and empty.
    pub fn dir(tag: &str) -> Self {
        let scratch = Self::new(tag);
        std::fs::create_dir_all(&scratch.0).expect("could not create scratch directory");
        scratch
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    fn remove(&self) {
        // Either shape is possible and both are fine if absent, so neither
        // result is checked - this runs during unwind, where a panic would
        // abort the process.
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl AsRef<Path> for ScratchPath {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

/// Same shape as `PathBuf` derefs to `Path`, so a scratch path can be passed and
/// joined exactly like the `PathBuf` it replaces.
impl std::ops::Deref for ScratchPath {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchPath {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_itself_when_the_scope_ends() {
        let recorded;
        {
            let scratch = ScratchPath::dir("selftest_scope");
            recorded = scratch.path().to_path_buf();
            std::fs::write(scratch.join("f.txt"), b"x").unwrap();
            assert!(recorded.is_dir());
        }
        assert!(
            !recorded.exists(),
            "the scratch directory outlived its scope"
        );
    }

    /// The property the trailing-cleanup pattern does not have.
    #[test]
    fn removes_itself_even_when_the_test_panics() {
        let scratch = ScratchPath::dir("selftest_panic");
        let recorded = scratch.path().to_path_buf();
        std::fs::write(scratch.join("f.txt"), b"x").unwrap();

        let result = std::panic::catch_unwind(move || {
            let _owned = scratch;
            panic!("simulated assertion failure");
        });

        assert!(result.is_err(), "the simulated failure did not panic");
        assert!(
            !recorded.exists(),
            "the scratch directory survived a failing test"
        );
    }

    /// Two tests deriving a name from the same tag must not collide, which is
    /// what the fixed `dat3_*_test.dat` filenames used to risk.
    #[test]
    fn two_scratch_paths_with_one_tag_differ() {
        let a = ScratchPath::new("selftest_unique");
        let b = ScratchPath::new("selftest_unique");
        assert_ne!(a.path(), b.path());
    }
}
