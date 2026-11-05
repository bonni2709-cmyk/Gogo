//! Десктопная точка входа. На Android собирается `cdylib`
//! (`src/lib.rs`, символ `quad_main`), бинарник здесь — только для ПК.

fn main() {
    voxel_knight::run();
}
