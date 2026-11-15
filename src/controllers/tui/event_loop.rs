use std::{sync::Arc, time::Duration};

use ratatui::backend::Backend;

use crate::{
    controllers::tui::{composer::Composer, renderer::Renderer},
    entities::{
        agent_command::command::AgentCommand, agent_event::event::AgentEvent,
        application::input_event::InputEvent,
    },
    infrastructure::ui::{
        gateway::{AgentEventSource, AgentGateway},
        input::InputSource,
        presenter::UiStateMachine,
    },
};

/// Boundary implementations and UI state machine supplied by the composition root.
pub(crate) struct TuiDependencies {
    /// Terminal input stream.
    pub(crate) input: Arc<dyn InputSource>,
    /// Agent event stream.
    pub(crate) events: Arc<dyn AgentEventSource>,
    /// Command sender.
    pub(crate) gateway: Arc<dyn AgentGateway>,
    /// UI state machine owned by the use-case layer.
    pub(crate) workflow: Box<dyn UiStateMachine>,
}

/// Runs the in-process event loop until the agent or the terminal ends.
pub(crate) struct TuiEventLoop<B: Backend> {
    input: Arc<dyn InputSource>,
    events: Arc<dyn AgentEventSource>,
    gateway: Arc<dyn AgentGateway>,
    workflow: Box<dyn UiStateMachine>,
    composer: Composer,
    renderer: Renderer<B>,
    dirty: bool,
}

impl<B: Backend> TuiEventLoop<B> {
    /// Creates the loop from the event boundary and one renderer.
    pub(crate) fn new(dependencies: TuiDependencies, renderer: Renderer<B>) -> Self {
        Self {
            input: dependencies.input,
            events: dependencies.events,
            gateway: dependencies.gateway,
            workflow: dependencies.workflow,
            composer: Composer::new(),
            renderer,
            dirty: true,
        }
    }

    /// Draws the first frame and processes input until shutdown.
    pub(crate) async fn run(&mut self) -> anyhow::Result<()> {
        let mut ticker = tokio::time::interval(Duration::from_millis(33));
        loop {
            tokio::select! {
                input = self.input.next_event() => {
                    match input? {
                        Some(event) => self.handle_input(event),
                        None => {
                            self.gateway.send(AgentCommand::Exit);
                            break;
                        }
                    }
                    self.drain_events();
                }
                event = self.events.next_event() => {
                    match event {
                        Some(event) => self.apply_event(event),
                        None => break,
                    }
                    self.drain_events();
                }
                _ = ticker.tick() => {
                    if self.dirty {
                        self.render()?;
                        self.dirty = false;
                    }
                }
            }
            if self.workflow.shutdown() {
                break;
            }
        }
        self.render()?;
        Ok(())
    }

    fn handle_input(&mut self, event: InputEvent) {
        let draft = self.composer.text();
        let result = self.workflow.handle_input(event.clone(), draft.as_str());
        if result.forward_to_composer {
            self.composer.apply(&event);
        }
        if let Some(intent) = result.scroll {
            self.renderer.scroll(intent);
        }
        self.send(result.commands);
        if result.clear_composer {
            self.composer.clear();
        }
        self.dirty = true;
    }

    fn apply_event(&mut self, event: AgentEvent) {
        let commands = self.workflow.apply_agent_event(event);
        self.send(commands);
        self.dirty = true;
    }

    fn drain_events(&mut self) {
        while let Some(event) = self.events.try_next() {
            self.apply_event(event);
        }
    }

    fn send(&self, commands: Vec<AgentCommand>) {
        for command in commands {
            self.gateway.send(command);
        }
    }

    fn render(&mut self) -> anyhow::Result<()> {
        let model = self.workflow.render_model();
        self.renderer.draw(&model, &mut self.composer)
    }
}
