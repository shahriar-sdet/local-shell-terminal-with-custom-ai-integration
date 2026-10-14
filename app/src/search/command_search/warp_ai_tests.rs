use std::collections::HashSet;
use std::sync::Arc;

use async_io::block_on;
use warpui::App;

use super::*;
use crate::search::QueryFilter;
use crate::search::mixer::{AsyncDataSource, SyncDataSource};
use crate::server::server_api::ai::MockAIClient;

fn natural_language_query(text: &str) -> Query {
    Query {
        filters: HashSet::from([QueryFilter::NaturalLanguage]),
        text: text.to_owned(),
    }
}

fn client_expecting_generations(times: usize) -> Arc<dyn AIClient> {
    let mut client = MockAIClient::new();
    client
        .expect_generate_commands_from_natural_language()
        .times(times)
        .returning(|_, _| Err(GenerateCommandsFromNaturalLanguageError::BadPrompt));
    Arc::new(client)
}

fn sync_actions(
    source: &WarpAIDataSource,
    query: &Query,
    app: &AppContext,
) -> Vec<CommandSearchItemAction> {
    SyncDataSource::run_query(source, query, app)
        .unwrap_or_else(|_| panic!("sync source should not fail"))
        .iter()
        .map(|result| result.accept_result())
        .collect()
}

#[test]
fn on_demand_source_offers_generation_without_calling_the_model() {
    App::test((), |app| async move {
        let request = GenerationRequest::default();
        let source =
            WarpAIDataSource::new(client_expecting_generations(0), None, Some(request.clone()));
        let query = natural_language_query("find files larger than 100 MB");

        app.read(|ctx| {
            assert!(matches!(
                sync_actions(&source, &query, ctx).as_slice(),
                [CommandSearchItemAction::GenerateWithAI]
            ));
            let results = block_on(AsyncDataSource::run_query(&source, &query, ctx));
            assert!(results.is_ok_and(|results| results.is_empty()));
        });
    });
}

#[test]
fn on_demand_source_generates_once_requested_for_that_text() {
    App::test((), |app| async move {
        let request = GenerationRequest::default();
        let source =
            WarpAIDataSource::new(client_expecting_generations(1), None, Some(request.clone()));
        let query = natural_language_query("find files larger than 100 MB");
        *request.lock() = Some(query.text.clone());

        app.read(|ctx| {
            assert!(sync_actions(&source, &query, ctx).is_empty());
            let results = block_on(AsyncDataSource::run_query(&source, &query, ctx));
            assert!(results.is_err());

            // Editing the text after asking goes back to offering a generation.
            let edited = natural_language_query("find files larger than 1 GB");
            assert!(matches!(
                sync_actions(&source, &edited, ctx).as_slice(),
                [CommandSearchItemAction::GenerateWithAI]
            ));
            let results = block_on(AsyncDataSource::run_query(&source, &edited, ctx));
            assert!(results.is_ok_and(|results| results.is_empty()));
        });
    });
}

#[test]
fn as_you_type_source_keeps_asking_and_generating_on_every_query() {
    App::test((), |app| async move {
        let source = WarpAIDataSource::new(client_expecting_generations(1), None, None);
        let query = natural_language_query("find files larger than 100 MB");

        app.read(|ctx| {
            assert!(matches!(
                sync_actions(&source, &query, ctx).as_slice(),
                [CommandSearchItemAction::OpenWarpAI]
            ));
            let results = block_on(AsyncDataSource::run_query(&source, &query, ctx));
            assert!(results.is_err());
        });
    });
}
