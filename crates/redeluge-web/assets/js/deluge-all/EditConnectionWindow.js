/**
 * Deluge.EditConnectionWindow.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.EditConnectionWindow
 * @extends Ext.Window
 */
Deluge.EditConnectionWindow = Ext.extend(Ext.Window, {
    title: _('Edit Connection'),
    iconCls: 'x-deluge-add-window-icon',

    layout: 'fit',
    width: 340,
    height: 235,
    constrainHeader: true,
    bodyStyle: 'padding: 10px 5px;',
    closeAction: 'hide',

    initComponent: function () {
        Deluge.EditConnectionWindow.superclass.initComponent.call(this);

        this.addEvents('hostedited');

        this.addButton(_('Close'), this.hide, this);
        this.addButton(_('Edit'), this.onEditClick, this);

        this.on('hide', this.onHide, this);

        this.form = this.add({
            xtype: 'form',
            defaultType: 'textfield',
            baseCls: 'x-plain',
            // Wide enough for "Pin sha256:", which wrapped onto two lines at
            // the 60 that suited the four captions above it.
            labelWidth: 75,
            items: [
                {
                    fieldLabel: _('Host:'),
                    labelSeparator: '',
                    name: 'host',
                    anchor: '75%',
                    value: '',
                },
                {
                    xtype: 'spinnerfield',
                    fieldLabel: _('Port:'),
                    labelSeparator: '',
                    name: 'port',
                    strategy: {
                        xtype: 'number',
                        decimalPrecision: 0,
                        minValue: 0,
                        maxValue: 65535,
                    },
                    anchor: '40%',
                    value: 58846,
                },
                {
                    fieldLabel: _('Username:'),
                    labelSeparator: '',
                    name: 'username',
                    anchor: '75%',
                    value: '',
                },
                {
                    fieldLabel: _('Password:'),
                    labelSeparator: '',
                    anchor: '75%',
                    name: 'password',
                    inputType: 'password',
                    value: '',
                },
                {
                    // The daemon prints its own certificate fingerprint when
                    // it starts. Pasting it here pins the certificate for this
                    // host; leaving it empty connects without verifying
                    // anything, which is what the Python client always did and
                    // is only reasonable over loopback.
                    fieldLabel: _('Pin sha256:'),
                    labelSeparator: '',
                    name: 'fingerprint',
                    // The same anchor as the fields above it. At 95% the box
                    // ran past the right edge of the window and was clipped;
                    // a sha256 is wider than any of these boxes anyway, so it
                    // scrolls inside whatever width it is given.
                    anchor: '75%',
                    emptyText: _('unpinned'),
                    value: '',
                },
            ],
        });
    },

    show: function (connection) {
        Deluge.EditConnectionWindow.superclass.show.call(this);

        this.form.getForm().findField('host').setValue(connection.get('host'));
        this.form.getForm().findField('port').setValue(connection.get('port'));
        this.form
            .getForm()
            .findField('username')
            .setValue(connection.get('user'));
        this.host_id = connection.id;

        // Pins live in the Web UI's own configuration, keyed by host id,
        // rather than in the host list: they are this server's opinion about
        // that daemon, not part of the address.
        var field = this.form.getForm().findField('fingerprint');
        field.setValue('');
        deluge.client.web.get_config({
            success: function (config) {
                var pins = config['daemon_fingerprints'] || {};
                field.setValue(pins[this.host_id] || '');
                this.pins = pins;
            },
            scope: this,
        });
    },

    onEditClick: function () {
        var values = this.form.getForm().getValues();
        deluge.client.web.edit_host(
            this.host_id,
            values.host,
            Number(values.port),
            values.username,
            values.password,
            {
                success: function (result) {
                    if (!result) {
                        console.log(result);
                        Ext.MessageBox.show({
                            title: _('Error'),
                            msg: String.format(_('Unable to edit host')),
                            buttons: Ext.MessageBox.OK,
                            modal: false,
                            icon: Ext.MessageBox.ERROR,
                            iconCls: 'x-deluge-icon-error',
                        });
                    } else {
                        this.saveFingerprint(values.fingerprint);
                        this.fireEvent('hostedited');
                    }
                    this.hide();
                },
                scope: this,
            }
        );
    },

    /**
     * Stores, or clears, the pinned fingerprint for the host being edited.
     */
    saveFingerprint: function (value) {
        var pins = Ext.apply({}, this.pins || {});
        var pin = (value || '').replace(/[\s:]/g, '').toLowerCase();
        if (pin) {
            pins[this.host_id] = pin;
        } else {
            delete pins[this.host_id];
        }
        deluge.client.web.set_config({ daemon_fingerprints: pins });
        this.pins = pins;
    },

    onHide: function () {
        this.form.getForm().reset();
    },
});
