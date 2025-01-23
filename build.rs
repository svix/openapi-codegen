use vergen_gitcl::{Emitter, GitclBuilder};

fn main() {
    let git2 = GitclBuilder::default().sha(false).build().unwrap();
    Emitter::new()
        .add_instructions(&git2)
        .unwrap()
        .emit()
        .unwrap();
}
