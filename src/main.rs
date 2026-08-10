use std::fs::File;

fn main() {
    let class_path = std::env::args().nth(1).expect("Expected class file");
    let mut class_file = File::open(class_path).expect("Failed to open class file");

    let class =
        janadinite::class::Class::decode(&mut class_file).expect("Failed to decode class file");

    println!("{class:#x?}");

    for meth in class.methods() {
        println!(
            "{:#x} {} {} {{",
            meth.access_flags(),
            meth.descriptor(),
            meth.name()
        );

        if let Some(code) = meth.code() {
            println!("locals={},stack={}", code.max_locals(), code.max_stack());
            for op in code.instructions() {
                println!("  {op:?}");
            }
        }

        println!("}}");
    }
}
