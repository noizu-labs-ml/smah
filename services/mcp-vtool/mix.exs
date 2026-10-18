defmodule McpVtool.MixProject do
  use Mix.Project

  def project do
    [
      app: :mcp_vtool,
      version: "0.1.0",
      elixir: "~> 1.18",
      elixirc_paths: elixirc_paths(Mix.env()),
      start_permanent: Mix.env() == :prod,
      deps: []
    ]
  end

  defp elixirc_paths(:test), do: ["lib", "test/support"]
  defp elixirc_paths(_), do: ["lib"]

  # Standalone frozen-artifact replay server for smah virtual MCP tools
  # (ticket #23). Zero hex dependencies: JSON rides the in-tree
  # `McpVtool.Json` codec so object key order survives the round trip
  # (the memo key canonicalization depends on it).
  def application do
    [
      extra_applications: [:logger, :crypto],
      mod: {McpVtool.Application, []}
    ]
  end
end
