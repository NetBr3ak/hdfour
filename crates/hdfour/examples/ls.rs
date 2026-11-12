use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: ls FILE")?;
    if arguments.next().is_some() {
        return Err("usage: ls FILE".into());
    }
    let file = hdfour::open(path)?;
    println!("File attributes: {:#?}", file.attributes());
    for (index, dataset) in file.datasets().iter().enumerate() {
        println!("Dataset {index}: {dataset:#?}");
    }
    println!("Skipped objects: {:#?}", file.skipped());
    println!("EOS metadata: {:#?}", file.eos());
    Ok(())
}
