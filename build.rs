use vergen_gitcl::{Emitter, GitclBuilder};

fn main() {
    let git = GitclBuilder::default().sha(false).build().unwrap();
    Emitter::new()
        .add_instructions(&git)
        .unwrap()
        .emit()
        .unwrap();
}
