/**
 * Ext JS Library 3.4.0
 * Copyright(c) 2006-2011 Sencha Inc.
 * licensing@sencha.com
 * http://www.sencha.com/license
 */
Ext.ns('Ext.ux.grid');

/**
 * @class Ext.ux.grid.BufferView
 * @extends Ext.grid.GridView
 * A custom GridView which renders rows on an as-needed basis.
 */
Ext.ux.grid.BufferView = Ext.extend(Ext.grid.GridView, {
    /**
     * @cfg {Number} rowHeight
     * The height of a row in the grid.
     */
    rowHeight: 19,

    /**
     * @cfg {Number} borderHeight
     * The combined height of border-top and border-bottom of a row.
     */
    borderHeight: 2,

    /**
     * @cfg {Boolean/Number} scrollDelay
     * The number of milliseconds before rendering rows out of the visible
     * viewing area. Defaults to 100. Rows will render immediately with a config
     * of false.
     */
    scrollDelay: 100,

    /**
     * @cfg {Number} cacheSize
     * The number of rows to look forward and backwards from the currently viewable
     * area.  The cache applies only to rows that have been rendered already.
     */
    cacheSize: 20,

    /**
     * @cfg {Number} cleanDelay
     * The number of milliseconds to buffer cleaning of extra rows not in the
     * cache.
     */
    cleanDelay: 500,

    initTemplates: function () {
        Ext.ux.grid.BufferView.superclass.initTemplates.call(this);
        var ts = this.templates;
        // empty div to act as a place holder for a row
        ts.rowHolder = new Ext.Template(
            '<div class="x-grid3-row {alt}" style="{tstyle}"></div>'
        );
        ts.rowHolder.disableFormats = true;
        ts.rowHolder.compile();

        ts.rowBody = new Ext.Template(
            '<table class="x-grid3-row-table" border="0" cellspacing="0" cellpadding="0" style="{tstyle}">',
            '<tbody><tr>{cells}</tr>',
            this.enableRowBody
                ? '<tr class="x-grid3-row-body-tr" style="{bodyStyle}"><td colspan="{cols}" class="x-grid3-body-cell" tabIndex="0" hidefocus="on"><div class="x-grid3-row-body">{body}</div></td></tr>'
                : '',
            '</tbody></table>'
        );
        ts.rowBody.disableFormats = true;
        ts.rowBody.compile();
    },

    getStyleRowHeight: function () {
        return Ext.isBorderBox
            ? this.rowHeight + this.borderHeight
            : this.rowHeight;
    },

    /**
     * How far apart two rows really are, in pixels.
     *
     * Diverges from the Ext JS original, which returns `rowHeight +
     * borderHeight` and trusts it. That constant is only right at a browser
     * zoom of exactly 100% with the stylesheet's own font size. Anywhere else
     * the real row is a fraction of a pixel taller or shorter, the error
     * accumulates down the list, and the window this drives stops covering
     * what is actually on screen: the last rows of the viewport are never
     * rendered and show as empty stripes. Measuring costs one layout read per
     * refresh and cannot drift.
     */
    getCalculatedRowHeight: function () {
        if (this.measuredRowHeight > 0) {
            return this.measuredRowHeight;
        }

        var rows = this.getRows();
        if (rows && rows.length > 1) {
            // Measured across the whole list rather than between two
            // neighbours, and with `getBoundingClientRect` rather than
            // `offsetTop`, because the latter rounds to whole pixels. At a
            // browser zoom of 110% a row is 28.4px; rounding that to 28 puts
            // the error back, just more slowly.
            var span = rows.length - 1;
            var top = rows[0].getBoundingClientRect().top;
            var bottom = rows[span].getBoundingClientRect().top;
            var pitch = (bottom - top) / span;
            if (pitch > 0) {
                this.measuredRowHeight = pitch;
                return pitch;
            }
        }
        if (rows && rows.length === 1) {
            var single = rows[0].getBoundingClientRect().height;
            if (single > 0) {
                this.measuredRowHeight = single;
                return single;
            }
        }

        // Nothing rendered yet. The configured value is the best guess, and it
        // is not cached, so the first real measurement still happens.
        return this.rowHeight + this.borderHeight;
    },

    /**
     * Forgets the measurement, so the next read takes it again.
     *
     * Called wherever the geometry can change: a resize, a zoom, a theme.
     */
    forgetRowHeight: function () {
        this.measuredRowHeight = null;
    },

    getVisibleRowCount: function () {
        var rh = this.getCalculatedRowHeight(),
            visibleHeight = this.scroller.dom.clientHeight;
        return visibleHeight < 1 ? 0 : Math.ceil(visibleHeight / rh);
    },

    getVisibleRows: function () {
        var count = this.getVisibleRowCount(),
            rh = this.getCalculatedRowHeight(),
            sc = this.scroller.dom.scrollTop,
            start = sc === 0 ? 0 : Math.floor(sc / rh) - 1,
            end = this.ds.getCount() - 1;

        // Clamped so the window is never inverted. When it was, `first` could
        // exceed `last` and every row in view rendered empty.
        var first = Math.max(Math.min(start, end), 0);
        // Four rows of slack rather than two: enough that a residual rounding
        // error still covers the bottom of the viewport.
        var last = Math.max(Math.min(start + count + 4, end), first);
        return { first: first, last: last };
    },

    doRender: function (cs, rs, ds, startRow, colCount, stripe, onlyBody) {
        var ts = this.templates,
            ct = ts.cell,
            rt = ts.row,
            rb = ts.rowBody,
            last = colCount - 1,
            rh = this.getStyleRowHeight(),
            vr = this.getVisibleRows(),
            tstyle = 'width:' + this.getTotalWidth() + ';height:' + rh + 'px;',
            // buffers
            buf = [],
            cb,
            c,
            p = {},
            rp = { tstyle: tstyle },
            r;
        for (var j = 0, len = rs.length; j < len; j++) {
            r = rs[j];
            cb = [];
            var rowIndex = j + startRow,
                visible = rowIndex >= vr.first && rowIndex <= vr.last;
            if (visible) {
                for (var i = 0; i < colCount; i++) {
                    c = cs[i];
                    p.id = c.id;
                    p.css =
                        i === 0
                            ? 'x-grid3-cell-first '
                            : i == last
                            ? 'x-grid3-cell-last '
                            : '';
                    p.attr = p.cellAttr = '';
                    // Before the renderer, not after. The Ext JS original set
                    // the style afterwards, which left every renderer looking
                    // at the *previous* column's style, because `p` is one
                    // object reused down the row. A renderer that sizes
                    // something to its column then got the wrong width: the
                    // progress bar came out as wide as the Size column beside
                    // it, about two fifths of its cell, with the percentage cut
                    // off inside it. The stock `Ext.grid.GridView` does it in
                    // this order; only this buffered subclass did not.
                    p.style = c.style;
                    p.value = c.renderer.call(
                        c.scope,
                        r.data[c.name],
                        p,
                        r,
                        rowIndex,
                        i,
                        ds
                    );
                    if (p.value === undefined || p.value === '') {
                        p.value = '&#160;';
                    }
                    if (r.dirty && typeof r.modified[c.name] !== 'undefined') {
                        p.css += ' x-grid3-dirty-cell';
                    }
                    cb[cb.length] = ct.apply(p);
                }
            }
            var alt = [];
            if (stripe && (rowIndex + 1) % 2 === 0) {
                alt[0] = 'x-grid3-row-alt';
            }
            if (r.dirty) {
                alt[1] = ' x-grid3-dirty-row';
            }
            rp.cols = colCount;
            if (this.getRowClass) {
                alt[2] = this.getRowClass(r, rowIndex, rp, ds);
            }
            rp.alt = alt.join(' ');
            rp.cells = cb.join('');
            buf[buf.length] = !visible
                ? ts.rowHolder.apply(rp)
                : onlyBody
                ? rb.apply(rp)
                : rt.apply(rp);
        }
        return buf.join('');
    },

    isRowRendered: function (index) {
        var row = this.getRow(index);
        return row && row.childNodes.length > 0;
    },

    syncScroll: function () {
        Ext.ux.grid.BufferView.superclass.syncScroll.apply(this, arguments);
        this.update();
    },

    // a (optionally) buffered method to update contents of gridview
    update: function () {
        if (this.scrollDelay) {
            if (!this.renderTask) {
                this.renderTask = new Ext.util.DelayedTask(this.doUpdate, this);
            }
            this.renderTask.delay(this.scrollDelay);
        } else {
            this.doUpdate();
        }
    },

    onRemove: function (ds, record, index, isUpdate) {
        Ext.ux.grid.BufferView.superclass.onRemove.apply(this, arguments);
        if (isUpdate !== true) {
            this.update();
        }
    },

    doUpdate: function () {
        if (this.getVisibleRowCount() > 0) {
            var g = this.grid,
                cm = g.colModel,
                ds = g.store,
                cs = this.getColumnData(),
                vr = this.getVisibleRows(),
                row;
            for (var i = vr.first; i <= vr.last; i++) {
                // if row is NOT rendered and is visible, render it
                if (!this.isRowRendered(i) && (row = this.getRow(i))) {
                    var html = this.doRender(
                        cs,
                        [ds.getAt(i)],
                        ds,
                        i,
                        cm.getColumnCount(),
                        g.stripeRows,
                        true
                    );
                    row.innerHTML = html;
                }
            }
            this.clean();
        }
    },

    // a buffered method to clean rows
    clean: function () {
        if (!this.cleanTask) {
            this.cleanTask = new Ext.util.DelayedTask(this.doClean, this);
        }
        this.cleanTask.delay(this.cleanDelay);
    },

    doClean: function () {
        if (this.getVisibleRowCount() > 0) {
            var vr = this.getVisibleRows();
            vr.first -= this.cacheSize;
            vr.last += this.cacheSize;

            var i = 0,
                rows = this.getRows();
            // if first is less than 0, all rows have been rendered
            // so lets clean the end...
            if (vr.first <= 0) {
                i = vr.last + 1;
            }
            // `len` is the smaller of the two: the store can hold more
            // records than the body holds rows for a moment after an add, and
            // indexing past the end threw from inside a timer.
            var len = Math.min(this.ds.getCount(), rows.length);
            for (; i < len; i++) {
                // if current row is outside of first and last and
                // has content, update the innerHTML to nothing
                if ((i < vr.first || i > vr.last) && rows[i].innerHTML) {
                    rows[i].innerHTML = '';
                }
            }
        }
    },

    removeTask: function (name) {
        var task = this[name];
        if (task && task.cancel) {
            task.cancel();
            this[name] = null;
        }
    },

    destroy: function () {
        this.removeTask('cleanTask');
        this.removeTask('renderTask');
        Ext.ux.grid.BufferView.superclass.destroy.call(this);
    },

    layout: function () {
        Ext.ux.grid.BufferView.superclass.layout.call(this);
        // The grid has just been sized, so any earlier measurement is stale.
        this.forgetRowHeight();
        this.update();
    },
});
