# Copyright 2026 OpenC3, Inc.
# All Rights Reserved.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.
# See LICENSE.md for more details.

# frozen_string_literal: true

module ERBLite
  # AST node types. Every node carries the 1-based template line it came from.
  module Nodes
    Text      = Struct.new(:text, :line)                       # literal template text
    Output    = Struct.new(:expr, :line)                       # <%= expr %>
    Seq       = Struct.new(:stmts, :line)                      # statement list
    Lit       = Struct.new(:value, :line)                      # nil/true/false/Integer/Float/Symbol
    Str       = Struct.new(:parts, :line)                      # parts: String | node
    DSym      = Struct.new(:parts, :line)                      # :"..."
    Arr       = Struct.new(:elems, :line)
    HashLit   = Struct.new(:pairs, :line)                      # [[key_node, value_node], ...]
    RangeLit  = Struct.new(:lo, :hi, :excl, :line)
    LVar      = Struct.new(:name, :depth, :line)               # local variable read
    LAsgn     = Struct.new(:name, :depth, :value, :line)
    MAsgn     = Struct.new(:targets, :values, :line)           # targets: [[name, depth]]
    OpAsgn    = Struct.new(:target, :op, :value, :line)        # a op= b, a ||= b, h[k] += 1
    IndexAsgn = Struct.new(:recv, :args, :value, :line)        # a[i] = v
    AttrAsgn  = Struct.new(:recv, :name, :value, :line)        # a.b = v
    Call      = Struct.new(:recv, :name, :args, :block, :safe, :line)
    FCall     = Struct.new(:name, :args, :block, :line)        # receiverless call: format(...)
    VCall     = Struct.new(:name, :line)                       # bare identifier, not a local
    Const     = Struct.new(:name, :line)
    Colon2    = Struct.new(:scope, :name, :line)               # Math::PI
    And       = Struct.new(:left, :right, :line)
    Or        = Struct.new(:left, :right, :line)
    Not       = Struct.new(:expr, :line)
    If        = Struct.new(:cond, :thn, :els, :line)
    While     = Struct.new(:cond, :body, :neg, :line)          # neg: until
    For       = Struct.new(:vars, :iter, :body, :line)         # vars: [[name, depth]]
    Case      = Struct.new(:subject, :whens, :els, :line)      # whens: [[conds, body]]
    Break     = Struct.new(:value, :kind, :line)               # kind: :loop or :block
    Next      = Struct.new(:value, :kind, :line)
    BlockNode = Struct.new(:params, :body, :line)              # params: String | nested Array
    BlockPass = Struct.new(:name, :line)                       # &:sym
  end
end
