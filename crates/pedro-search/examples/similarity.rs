//! Prints how alike the model thinks two pieces of text are.
//!
//! ```bash
//! cargo run -p pedro-search --example similarity
//! ```
//!
//! Used to choose the floor below which a passage is not worth attaching to a
//! question: the numbers a floor has to separate are the ones this prints.

use pedro_search::embed::Embedder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Embedder::find().ok_or("no model; run scripts/fetch-embedding.sh")?;

    let pairs = [
        ("素数を生成する方法", "素数を生成するアルゴリズム"),
        ("素数を生成する方法", "乱数から素数を選ぶ手順について述べる"),
        ("素数を生成する方法", "鍵長は安全性の見積もりから決まる"),
        ("どう違う?", "page18"),
        ("どう違う?", "この二つの方式の違いを説明する"),
        ("どう違う?", "はじめに"),
        ("鍵長はどう推定する?", "第3章 まえがき"),
        ("鍵長はどう推定する?", "鍵長の推定は計算量の見積もりに基づく"),
    ];

    for (question, passage) in pairs {
        let a = embedder.embed(question)?;
        let b = embedder.embed(passage)?;
        let cosine: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();

        println!("{cosine:.3}  {question}  ~  {passage}");
    }

    Ok(())
}
