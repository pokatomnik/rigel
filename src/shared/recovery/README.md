# Recovery

This module handles recoverable failures during model turns. It formats
streaming and prompt errors, tracks invalid tool calls, and retries turns when
the model can correct its previous response.
