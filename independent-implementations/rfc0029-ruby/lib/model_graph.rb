# frozen_string_literal: true

require_relative "expand"

# Lightweight local (non-catalog-resolved) graph view of one fixture: just
# {local_id => kind} nodes and [source, kind, target] edges, used by the
# semantic evaluators. Kept separate from Expand's canonical-JSON expansion
# so the semantic computation never depends on catalog/oracle lookups.
module ModelGraph
  Graph = Struct.new(:nodes, :edges, :out_edges, :in_edges)

  def self.build(fixture)
    nodes = {}
    (fixture["nodes"] || []).each do |tok|
      id, kind = Expand.parse_node_token(tok)
      nodes[id] = kind
    end
    edges = (fixture["edges"] || []).map { |tok| Expand.parse_edge_token(tok) }
    out_edges = Hash.new { |h, k| h[k] = [] }
    in_edges = Hash.new { |h, k| h[k] = [] }
    edges.each do |src, kind, tgt|
      out_edges[src] << [kind, tgt]
      in_edges[tgt] << [src, kind]
    end
    Graph.new(nodes, edges, out_edges, in_edges)
  end

  def self.topo_order(graph)
    indeg = Hash.new(0)
    graph.nodes.each_key { |n| indeg[n] = graph.in_edges[n].size }
    queue = graph.nodes.each_key.select { |n| indeg[n].zero? }
    order = []
    seen_edges = Hash.new(0)
    until queue.empty?
      n = queue.shift
      order << n
      graph.out_edges[n].each do |_, tgt|
        seen_edges[tgt] += 1
        queue << tgt if seen_edges[tgt] == graph.in_edges[tgt].size
      end
    end
    order
  end

  def self.leaf?(graph, id)
    graph.out_edges[id].empty?
  end
end
