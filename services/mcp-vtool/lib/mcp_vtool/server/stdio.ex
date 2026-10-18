defmodule McpVtool.Server.Stdio do
  @moduledoc """
  Newline-delimited JSON-RPC 2.0 over stdin/stdout — the same batch/line
  surface the Rust lane serves by default, for piping one request set
  through the replay service. Started only when `MCPVTOOL_STDIO=1`, so
  `mix run`/`mix test` never fight the test runner for stdin. Reads to
  EOF, answers each request in order, then exits normally.
  """

  use GenServer

  alias McpVtool.Handler

  def start_link(args) do
    GenServer.start_link(__MODULE__, args, name: __MODULE__)
  end

  @impl true
  def init(args) do
    config =
      case args do
        [] -> McpVtool.Tools.config()
        config when is_map(config) -> Map.merge(McpVtool.Tools.config(), config)
      end

    {:ok, config, {:continue, :loop}}
  end

  @impl true
  def handle_continue(:loop, config) do
    case IO.read(:stdio, :line) do
      :eof ->
        {:stop, :normal, config}

      {:error, reason} ->
        {:stop, reason, config}

      line ->
        response = Handler.handle_line(line, config)

        if response != "" do
          IO.write(response)
        end

        {:noreply, config, {:continue, :loop}}
    end
  end
end
