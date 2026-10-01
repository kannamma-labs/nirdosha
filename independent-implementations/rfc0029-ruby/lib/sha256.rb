# frozen_string_literal: true

require "digest"

# Thin wrapper over Ruby's standard-library Digest::SHA256. The hashing
# primitive itself is explicitly allowed by the task ("yaml, json, digest are
# all you should need"); only the JCS canonicalization above it is
# hand-written.
module SHA256
  def self.hexdigest(bytes)
    Digest::SHA256.hexdigest(bytes)
  end
end
