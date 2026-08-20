//! Prints the agent CLIs pedro can find on this machine.
//!
//! ```sh
//! cargo run -p pedro-agent --example detect
//! ```

fn main() {
    let agents = pedro_agent::discover();

    if agents.is_empty() {
        println!("no agent CLI found");
        return;
    }

    for agent in agents {
        println!(
            "{:<12} {}  ({})",
            agent.kind.display_name(),
            agent.program.display(),
            agent.version.as_deref().unwrap_or("version unknown"),
        );
    }
}
