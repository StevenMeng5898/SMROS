R1 fifo ignores offset. R2 fifo's wrap arm returns the unwrapped base.
R3 socket ignores offset. R4 fifo is correct only when read_pos == 0.
R5 fifo ignores read_pos. R6 fifo is correct only when capacity == 2.
R7 socket returns capacity - 1 at every other point.
Each diff is only that mutation. The log says postcondition not satisfied.
The ensures are out == fifo_ring_index_spec / socket_ring_index_spec.
