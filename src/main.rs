use std::path::PathBuf;

use clap::Parser;

use crate::vm::VM;

pub mod vm;

#[derive(clap::Parser, Debug)]
struct Args {
    #[arg(long, short)]
    classpath: PathBuf,
    main_class: String,
}

fn main() {
    let args = Args::parse();

    let classpath = args.classpath;
    let main_class = args.main_class;

    vm::heap::register_thread();
    std::thread::spawn(vm::heap::gc_thread);

    let vm = VM::open(classpath, &*main_class).expect("Failed to open Class");
    println!("main() => {:?}", vm.run_main());
}
